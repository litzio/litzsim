use crate::circuit::StagedFile;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::Path;

const MAGIC: &[u8; 8] = b"LITZWV02";
const HEADER_SIZE: u64 = 20;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Channel {
    pub name: String,
    pub unit: String,
}

#[derive(Debug)]
pub struct Waveform {
    channels: Vec<Channel>,
    samples: Vec<f64>,
}

impl Waveform {
    pub(crate) fn new(signals: impl IntoIterator<Item = Channel>) -> Self {
        let time = Channel {
            name: "time".into(),
            unit: "s".into(),
        };
        Self {
            channels: std::iter::once(time).chain(signals).collect(),
            samples: Vec::new(),
        }
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[f64]> + DoubleEndedIterator {
        self.samples.chunks_exact(self.channels.len())
    }

    pub(crate) fn push(&mut self, time: f64, signals: impl Iterator<Item = f64>) {
        self.samples.push(time);
        self.samples.extend(signals);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_a_single_boundary_peak_and_its_immediate_neighbors() {
        let mut wave = Waveform::new([Channel {
            name: "probe".into(),
            unit: "V".into(),
        }]);
        for row in 0..512 {
            let value = match row {
                255 => 1.0,
                256 => 23.0,
                257 => 2.0,
                _ => 0.0,
            };
            wave.push(f64::from(row), [value].into_iter());
        }
        let channels = ["probe".to_string()];
        for (from, to) in [(0.0, 512.0), (254.0, 258.0)] {
            let view = wave.view(&channels, from, to, 2).unwrap();
            let series = &view.channels[0];
            assert_eq!(series.minimum, Some(0.0));
            assert_eq!(series.maximum, Some(23.0));
            assert!(
                series
                    .points
                    .windows(2)
                    .all(|points| points[0].row < points[1].row)
            );
            let around_peak: Vec<_> = series
                .points
                .iter()
                .filter(|point| (255..=257).contains(&point.row))
                .map(|point| (point.row, point.t_s, point.value))
                .collect();
            assert_eq!(
                around_peak,
                [(255, 255.0, 1.0), (256, 256.0, 23.0), (257, 257.0, 2.0)]
            );
        }
        let empty = wave.view(&channels, 512.0, 513.0, 2).unwrap();
        assert!(empty.channels[0].points.is_empty());
        assert_eq!(
            (empty.channels[0].minimum, empty.channels[0].maximum),
            (None, None)
        );
        let exact = wave.view(&channels, 256.0, 256.0, 1).unwrap();
        assert_eq!(exact.channels[0].points[0].row, 256);
        let clipped = wave.view(&channels, 256.0, 260.0, 1).unwrap();
        assert!(
            clipped.channels[0]
                .points
                .iter()
                .any(|point| point.row == 257)
        );
        assert!(
            clipped.channels[0]
                .points
                .iter()
                .all(|point| point.row >= 256)
        );
    }

    fn wave(values: impl Fn(usize) -> f64, rows: usize) -> Waveform {
        let mut wave = Waveform::new([Channel {
            name: "probe".into(),
            unit: "V".into(),
        }]);
        for row in 0..rows {
            wave.push(row as f64, [values(row)].into_iter());
        }
        wave
    }

    #[test]
    fn measures_the_lines_joining_original_rows_between_two_cursors() {
        let channels = ["probe".to_string()];
        let close = |a: f64, b: f64| (a - b).abs() < 1e-12;

        // v = t: the mean of t over 2.5..7.5 is 5 and the mean of t^2 is
        // (7.5^3 - 2.5^3) / 3 / 5 = 27.083..., independent of the rows.
        let ramp = wave(|row| row as f64, 11);
        let measured = ramp.measure(&channels, 2.5, 7.5).unwrap();
        let stats = &measured.channels[0];
        assert!(close(stats.average.unwrap(), 5.0));
        assert!(close(stats.rms.unwrap(), (406.25_f64 / 15.0).sqrt()));
        assert_eq!((stats.minimum, stats.maximum), (Some(2.5), Some(7.5)));
        // Cursors report the nearest original rows, the earlier one on a tie.
        assert_eq!((stats.from.row, stats.from.value), (2, 2.0));
        assert_eq!((stats.to.row, stats.to.value), (7, 7.0));
        assert_eq!(
            ramp.measure(&channels, 2.6, 2.6).unwrap().channels[0]
                .from
                .row,
            3
        );

        // Alternating 0, 1 rows join into a triangle wave: mean 1/2, RMS sqrt(1/3).
        let triangle = wave(|row| (row % 2) as f64, 101);
        let stats = &triangle.measure(&channels, 0.0, 100.0).unwrap().channels[0];
        assert!(close(stats.average.unwrap(), 0.5));
        assert!(close(stats.rms.unwrap(), (1.0_f64 / 3.0).sqrt()));

        // A single-row peak inside the interval sets the maximum.
        let peak = wave(|row| if row == 256 { 23.0 } else { 0.0 }, 512);
        let stats = &peak.measure(&channels, 100.5, 400.5).unwrap().channels[0];
        assert_eq!((stats.minimum, stats.maximum), (Some(0.0), Some(23.0)));
        assert!(close(stats.average.unwrap(), 23.0 / 300.0));

        // One cursor time: the interpolated value. Outside the result: no statistics,
        // and the cursors clamp to the first or last row.
        let stats = &ramp.measure(&channels, 4.25, 4.25).unwrap().channels[0];
        assert_eq!((stats.average, stats.rms), (Some(4.25), Some(4.25)));
        let outside = &ramp.measure(&channels, 20.0, 30.0).unwrap().channels[0];
        assert_eq!((outside.average, outside.maximum), (None, None));
        assert_eq!((outside.from.row, outside.to.row), (10, 10));
        let clipped = &ramp.measure(&channels, -10.0, 2.0).unwrap().channels[0];
        assert!(close(clipped.average.unwrap(), 1.0));
        assert_eq!(clipped.from.row, 0);

        // Cursors in either order: the same interval, signed differences.
        let reversed = ramp.measure(&channels, 7.5, 2.5).unwrap();
        let (forward, backward) = (&measured.channels[0], &reversed.channels[0]);
        assert_eq!(
            (backward.average, backward.rms),
            (forward.average, forward.rms)
        );
        assert_eq!((forward.delta, backward.delta), (5.0, -5.0));
        assert_eq!((measured.delta_time, reversed.delta_time), (5.0, -5.0));
        assert_eq!(
            (measured.frequency, reversed.frequency),
            (Some(0.2), Some(0.2))
        );
        assert_eq!(ramp.measure(&channels, 3.0, 3.0).unwrap().frequency, None);
        assert!(ramp.measure(&channels, f64::NAN, 2.0).is_err());
        assert!(ramp.measure(&["missing".into()], 0.0, 1.0).is_err());
        assert!(ramp.measure(&[], 0.0, 1.0).is_err());
    }

    #[test]
    fn two_rows_at_one_time_are_a_discontinuity() {
        // 0 V until t = 2, then 1 V: rows (2, 0) and (2, 1) are the left and right limits.
        let mut step = Waveform::new([Channel {
            name: "probe".into(),
            unit: "V".into(),
        }]);
        let samples = [
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (3.0, 1.0),
            (4.0, 1.0),
        ];
        for (t, v) in samples {
            step.push(t, [v].into_iter());
        }
        let channels = ["probe".to_string()];
        let measure = |from, to| {
            let mut measured = step.measure(&channels, from, to).unwrap();
            measured.channels.remove(0)
        };
        // Across the edge: half the interval at 1 V; the extremes are both limits.
        let across = measure(1.5, 2.5);
        assert_eq!(across.average, Some(0.5));
        assert!((across.rms.unwrap() - 0.5_f64.sqrt()).abs() < 1e-15);
        assert_eq!((across.minimum, across.maximum), (Some(0.0), Some(1.0)));
        // Ending on the edge takes its left limit, starting there its right limit.
        let before = measure(1.0, 2.0);
        assert_eq!((before.average, before.maximum), (Some(0.0), Some(0.0)));
        let after = measure(2.0, 3.0);
        assert_eq!((after.average, after.minimum), (Some(1.0), Some(1.0)));
        // Cursors: exactly on the edge read the left row, just after it the right row.
        let on = measure(2.0, 2.0);
        assert_eq!((on.from.row, on.average), (2, Some(0.0)));
        assert_eq!(measure(2.1, 2.1).from.row, 3);
        // The envelope keeps both rows, so the plot draws a vertical edge.
        let view = step.view(&channels, 0.0, 4.0, 4).unwrap();
        let rows: Vec<_> = view.channels[0]
            .points
            .iter()
            .map(|point| point.row)
            .collect();
        assert!(rows.contains(&2) && rows.contains(&3), "{rows:?}");

        // Files keep the pair; a third row at one time is refused.
        let path =
            std::env::temp_dir().join(format!("litzsim-edge-{}.litzwave", std::process::id()));
        let _ = fs::remove_file(&path);
        stage(&path, &step).unwrap();
        assert_eq!(read(&path).unwrap().rows().len(), 6);
        fs::remove_file(&path).unwrap();
        let mut triple = Waveform::new(step.channels()[1..].iter().cloned());
        for (t, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (1.0, 2.0)] {
            triple.push(t, [v].into_iter());
        }
        stage(&path, &triple).unwrap();
        assert!(read(&path).is_err());
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn summary_reads_rows_time_range_and_signals_without_the_samples() {
        let path =
            std::env::temp_dir().join(format!("litzsim-summary-{}.litzwave", std::process::id()));
        let _ = fs::remove_file(&path);
        let mut ramp = wave(|row| row as f64, 11);
        ramp.push(12.5, [1.0].into_iter());
        stage(&path, &ramp).unwrap();
        let summary = summary(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(summary.rows, 12);
        assert_eq!(summary.time_range, [0.0, 12.5]);
        assert_eq!(
            summary.channels,
            [Channel {
                name: "probe".into(),
                unit: "V".into()
            }]
        );
    }
}

/// Publish a complete result at a new path. An existing file is preserved.
pub fn write(path: &Path, wave: &Waveform) -> io::Result<()> {
    let staged = StagedFile::write(path, |file| encode(file, wave))?;
    fs::hard_link(&staged.0, path)
}

/// Write to a private staging path whose owner will publish or remove it.
pub fn stage(path: &Path, wave: &Waveform) -> io::Result<()> {
    let mut file = File::create_new(path)?;
    encode(&mut file, wave)?;
    file.sync_all()
}

fn encode(file: &mut File, wave: &Waveform) -> io::Result<()> {
    let channels = serde_json::to_vec(wave.channels())?;
    let length = u32::try_from(channels.len()).map_err(|_| invalid("channel header too large"))?;
    let rows = wave.rows().len() as u64;
    let mut out = BufWriter::new(file);
    out.write_all(MAGIC)?;
    out.write_all(&rows.to_le_bytes())?;
    out.write_all(&length.to_le_bytes())?;
    out.write_all(&channels)?;
    for sample in &wave.samples {
        out.write_all(&sample.to_le_bytes())?;
    }
    out.flush()
}

struct Header {
    rows: u64,
    channels: Vec<Channel>,
}

/// Check the signature, sizes and time channel, leaving `input` at the first sample.
fn read_header(input: &mut impl Read, file_size: u64) -> io::Result<Header> {
    if &read_bytes::<8>(input)? != MAGIC {
        return Err(invalid("invalid waveform signature"));
    }
    let rows = u64::from_le_bytes(read_bytes(input)?);
    let length = u32::from_le_bytes(read_bytes(input)?) as u64;
    let data_size = file_size
        .checked_sub(HEADER_SIZE + length)
        .filter(|size| rows > 0 && *size / 8 >= rows)
        .ok_or_else(|| invalid("invalid waveform length"))?;
    let channels: Vec<Channel> = serde_json::from_reader(input.take(length))?;
    if !channels
        .first()
        .is_some_and(|time| time.name == "time" && time.unit == "s")
    {
        return Err(invalid("invalid time channel"));
    }
    rows.checked_mul(channels.len() as u64)
        .filter(|count| count.checked_mul(8) == Some(data_size))
        .ok_or_else(|| invalid("invalid waveform row size"))?;
    Ok(Header { rows, channels })
}

/// Read all original rows. Signal values remain in their stored order.
pub fn read(path: &Path) -> io::Result<Waveform> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut input = BufReader::new(file);
    let Header { rows, channels } = read_header(&mut input, file_size)?;
    let count = usize::try_from(rows * channels.len() as u64)
        .map_err(|_| invalid("waveform too large to read into memory"))?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(count)
        .map_err(|_| invalid("waveform too large to read into memory"))?;
    for _ in 0..count {
        let value = f64::from_le_bytes(read_bytes(&mut input)?);
        if !value.is_finite() {
            return Err(invalid("non-finite waveform value"));
        }
        samples.push(value);
    }
    let wave = Waveform { channels, samples };
    // Two rows at the same time are the left and right limits of a discontinuity.
    let mut last = (f64::NEG_INFINITY, false);
    for row in wave.rows() {
        let repeated = row[0] == last.0;
        if row[0] < last.0 || (repeated && last.1) {
            return Err(invalid(
                "waveform time must increase, with at most two rows at one time",
            ));
        }
        last = (row[0], repeated);
    }
    Ok(wave)
}

/// Rows, time range and signals of a result, read from its header and first and last rows only.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub rows: u64,
    pub time_range: [f64; 2],
    pub channels: Vec<Channel>,
}

pub fn summary(path: &Path) -> io::Result<Summary> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut input = BufReader::new(&mut file);
    let Header { rows, mut channels } = read_header(&mut input, file_size)?;
    let first = f64::from_le_bytes(read_bytes(&mut input)?);
    drop(input);
    let row_size = channels.len() as u64 * 8;
    // The last row ends the file.
    file.seek(SeekFrom::Start(file_size - row_size))?;
    let last = f64::from_le_bytes(read_bytes(&mut file)?);
    channels.remove(0);
    Ok(Summary {
        rows,
        time_range: [first, last],
        channels,
    })
}

fn read_bytes<const N: usize>(input: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View<'a> {
    pub time_range: [f64; 2],
    pub channels: Vec<Series<'a>>,
}

#[derive(Serialize)]
pub struct Series<'a> {
    pub name: &'a str,
    pub unit: &'a str,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub points: Vec<Point>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement<'a> {
    pub time_range: [f64; 2],
    /// `to - from`; negative when the second cursor is earlier.
    #[serde(rename = "deltaTime_s")]
    pub delta_time: f64,
    /// `1 / |to - from|`, null when the cursors coincide.
    #[serde(rename = "frequency_Hz")]
    pub frequency: Option<f64>,
    pub channels: Vec<Statistics<'a>>,
}

/// Cursor rows and interval statistics; the statistics are null when the
/// interval lies outside the result.
#[derive(Serialize)]
pub struct Statistics<'a> {
    pub name: &'a str,
    pub unit: &'a str,
    pub from: Point,
    pub to: Point,
    /// `to.value - from.value`.
    pub delta: f64,
    pub average: Option<f64>,
    pub rms: Option<f64>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Point {
    pub row: usize,
    pub t_s: f64,
    pub value: f64,
}

impl Waveform {
    pub fn view(
        &self,
        channels: &[String],
        from: f64,
        to: f64,
        pixels: u32,
    ) -> io::Result<View<'_>> {
        let width = to - from;
        if !width.is_finite() || width < 0.0 || pixels == 0 || channels.is_empty() {
            return Err(invalid(
                "Expected a finite time range, positive pixels and channels",
            ));
        }
        let start = self.time_bound(|t| t < from);
        let end = self.time_bound(|t| t <= to);
        let columns = self.columns(channels)?;
        let mut series: Vec<_> = columns
            .iter()
            .map(|&column| Series {
                name: &self.channels[column].name,
                unit: &self.channels[column].unit,
                minimum: None,
                maximum: None,
                points: Vec::new(),
            })
            .collect();

        let pixel_width = width / f64::from(pixels);
        let edge = |pixel| from + f64::from(pixel) * pixel_width;
        let bucket = |t| {
            // Compare against actual split times so rounding a normalized
            // fraction cannot move a sample on an edge into an earlier bucket.
            let (mut first, mut last) = (0, pixels - 1);
            while first < last {
                let middle = first + (last - first) / 2 + 1;
                if t < edge(middle) {
                    last = middle - 1;
                } else {
                    first = middle;
                }
            }
            first
        };
        let mut begin = start;
        let mut next_edge = f64::NEG_INFINITY;
        let mut extremes = vec![(start, start); columns.len()];
        for (row, values) in (start..end).zip(self.rows().skip(start)) {
            if values[0] >= next_edge {
                if row > begin {
                    self.emit(start..end, &columns, &extremes, &mut series);
                }
                begin = row;
                extremes.fill((row, row));
                let next = bucket(values[0]) + 1;
                next_edge = if next < pixels {
                    edge(next)
                } else {
                    f64::INFINITY
                };
            } else {
                for (&column, (low, high)) in columns.iter().zip(&mut extremes) {
                    if values[column] < self.row(*low)[column] {
                        *low = row;
                    }
                    if values[column] > self.row(*high)[column] {
                        *high = row;
                    }
                }
            }
        }
        if begin < end {
            self.emit(start..end, &columns, &extremes, &mut series);
            for (&column, series) in columns.iter().zip(&mut series) {
                self.push_point(series, end - 1, column);
            }
        }
        Ok(View {
            time_range: [from, to],
            channels: series,
        })
    }

    /// Values at two cursor times, in either order, their differences and
    /// statistics of the interval between them.
    ///
    /// Each cursor reports the original row nearest to its time (the earlier
    /// row on a tie). The statistics treat the signal as the straight lines
    /// joining original rows, as the plot draws it, over the part of
    /// `from..=to` covered by the result: the time average, RMS, minimum and
    /// maximum, with interpolated values at both ends of the interval.
    pub fn measure(&self, channels: &[String], from: f64, to: f64) -> io::Result<Measurement<'_>> {
        if !from.is_finite() || !to.is_finite() || channels.is_empty() {
            return Err(invalid("Expected finite cursor times and channels"));
        }
        let last = self.rows().len() - 1;
        let (low, high) = (
            from.min(to).max(self.row(0)[0]),
            from.max(to).min(self.row(last)[0]),
        );
        let (from_row, to_row) = (self.nearest(from), self.nearest(to));
        let channels = self
            .columns(channels)?
            .into_iter()
            .map(|column| {
                let point = |row| Point {
                    row,
                    t_s: self.row(row)[0],
                    value: self.row(row)[column],
                };
                let interval = (low <= high).then(|| self.interval(column, low, high));
                let (from, to) = (point(from_row), point(to_row));
                Statistics {
                    name: &self.channels[column].name,
                    unit: &self.channels[column].unit,
                    delta: to.value - from.value,
                    from,
                    to,
                    average: interval.map(|stats| stats.0),
                    rms: interval.map(|stats| stats.1),
                    minimum: interval.map(|stats| stats.2),
                    maximum: interval.map(|stats| stats.3),
                }
            })
            .collect();
        let delta_time = to - from;
        Ok(Measurement {
            time_range: [from, to],
            delta_time,
            frequency: (delta_time != 0.0).then(|| 1.0 / delta_time.abs()),
            channels,
        })
    }

    fn columns(&self, names: &[String]) -> io::Result<Vec<usize>> {
        let mut columns = Vec::with_capacity(names.len());
        for name in names {
            let column = self
                .channels
                .iter()
                .position(|ch| &ch.name == name)
                .ok_or_else(|| invalid("Unknown waveform channel"))?;
            if columns.contains(&column) {
                return Err(invalid("Duplicate waveform channel"));
            }
            columns.push(column);
        }
        Ok(columns)
    }

    fn nearest(&self, time: f64) -> usize {
        let after = self.time_bound(|t| t < time);
        if after == 0 {
            return 0;
        }
        if after == self.rows().len() {
            return after - 1;
        }
        let before = after - 1;
        if time - self.row(before)[0] <= self.row(after)[0] - time {
            before
        } else {
            after
        }
    }

    /// The value on the line between the original rows around `time`, inside
    /// the result. At a discontinuity (two rows at `time`) `after` picks the
    /// right limit and otherwise the left limit.
    fn value_at(&self, column: usize, time: f64, after: bool) -> f64 {
        let next = if after {
            self.time_bound(|t| t <= time)
        } else {
            self.time_bound(|t| t < time)
        };
        if next == self.rows().len() {
            return self.row(next - 1)[column];
        }
        if next == 0 || self.row(next)[0] == time {
            return self.row(next)[column];
        }
        let (a, b) = (self.row(next - 1), self.row(next));
        a[column] + (b[column] - a[column]) * (time - a[0]) / (b[0] - a[0])
    }

    /// Average, RMS, minimum and maximum of the joined rows over `low..=high`.
    /// The interval starts with the right limit and ends with the left limit
    /// of a discontinuity; its two rows add no area. A zero-width interval at
    /// a discontinuity takes the left limit, as the cursor rows do.
    fn interval(&self, column: usize, low: f64, high: f64) -> (f64, f64, f64, f64) {
        if low == high {
            let value = self.value_at(column, low, false);
            return (value, value.abs(), value, value);
        }
        let first = self.value_at(column, low, true);
        let inside = self.time_bound(|t| t <= low)..self.time_bound(|t| t < high);
        let knots = std::iter::once((low, first))
            .chain(inside.map(|row| (self.row(row)[0], self.row(row)[column])))
            .chain(std::iter::once((high, self.value_at(column, high, false))));
        let (mut area, mut square, mut minimum, mut maximum) = (0.0, 0.0, first, first);
        let mut previous = (low, first);
        for (t, value) in knots.skip(1) {
            let (h, a, b) = (t - previous.0, previous.1, value);
            area += h * (a + b) / 2.0;
            square += h * (a * a + a * b + b * b) / 3.0;
            minimum = minimum.min(value);
            maximum = maximum.max(value);
            previous = (t, value);
        }
        let width = high - low;
        (
            area / width,
            (square / width).max(0.0).sqrt(),
            minimum,
            maximum,
        )
    }

    fn row(&self, row: usize) -> &[f64] {
        let width = self.channels.len();
        &self.samples[row * width..][..width]
    }

    fn time_bound(&self, before: impl Fn(f64) -> bool) -> usize {
        let (mut low, mut high) = (0, self.rows().len());
        while low < high {
            let mid = low + (high - low) / 2;
            if before(self.row(mid)[0]) {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        low
    }

    fn emit(
        &self,
        visible: Range<usize>,
        columns: &[usize],
        extremes: &[(usize, usize)],
        series: &mut [Series<'_>],
    ) {
        for ((&column, &(low, high)), series) in columns.iter().zip(extremes).zip(series) {
            let minimum = self.row(low)[column];
            let maximum = self.row(high)[column];
            series.minimum = Some(series.minimum.map_or(minimum, |old| old.min(minimum)));
            series.maximum = Some(series.maximum.map_or(maximum, |old| old.max(maximum)));
            if series.points.is_empty() {
                self.push_point(series, visible.start, column);
            }
            for row in [low.min(high), low.max(high)] {
                // Include the immediate neighbors of a sharp peak, even when
                // one of the neighbors belongs to a different pixel bucket.
                // Two rows at one time are both sides of a discontinuity; a
                // peak there is judged against the rows around the pair.
                let rows = self.rows().len();
                let twin = |a: usize, b: usize| self.row(a)[0] == self.row(b)[0];
                let start = if row > 0 && twin(row - 1, row) {
                    row - 1
                } else {
                    row
                };
                let end = if row + 1 < rows && twin(row, row + 1) {
                    row + 1
                } else {
                    row
                };
                if start > 0 && end + 1 < rows {
                    let before = self.row(start - 1)[column];
                    let value = self.row(row)[column];
                    let after = self.row(end + 1)[column];
                    let level = self.row(start)[column] == self.row(end)[column];
                    if level
                        && ((value > before && value > after) || (value < before && value < after))
                    {
                        let first = (start - 1).max(visible.start);
                        let last = (end + 1).min(visible.end - 1);
                        for sample in first..=last {
                            self.push_point(series, sample, column);
                        }
                        continue;
                    }
                }
                self.push_point(series, row, column);
            }
        }
    }

    fn push_point(&self, series: &mut Series<'_>, row: usize, column: usize) {
        if series.points.last().is_none_or(|point| point.row < row) {
            let values = self.row(row);
            series.points.push(Point {
                row,
                t_s: values[0],
                value: values[column],
            });
        }
    }
}
