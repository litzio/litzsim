use crate::circuit::StagedFile;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
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

/// Read all original rows. Signal values remain in their stored order.
pub fn read(path: &Path) -> io::Result<Waveform> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let mut input = BufReader::new(file);
    if &read_bytes::<8>(&mut input)? != MAGIC {
        return Err(invalid("invalid waveform signature"));
    }
    let rows = u64::from_le_bytes(read_bytes(&mut input)?);
    let length = u32::from_le_bytes(read_bytes(&mut input)?) as u64;
    let data_size = file_size
        .checked_sub(HEADER_SIZE + length)
        .filter(|size| rows > 0 && *size / 8 >= rows)
        .ok_or_else(|| invalid("invalid waveform length"))?;
    let channels: Vec<Channel> = serde_json::from_reader((&mut input).take(length))?;
    if !channels
        .first()
        .is_some_and(|time| time.name == "time" && time.unit == "s")
    {
        return Err(invalid("invalid time channel"));
    }
    let count = rows
        .checked_mul(channels.len() as u64)
        .filter(|count| count.checked_mul(8) == Some(data_size))
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| invalid("invalid waveform row size"))?;
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
    let mut last_time = f64::NEG_INFINITY;
    for row in wave.rows() {
        if row[0] <= last_time {
            return Err(invalid("waveform time must strictly increase"));
        }
        last_time = row[0];
    }
    Ok(wave)
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
        let mut columns = Vec::with_capacity(channels.len());
        let mut series = Vec::with_capacity(channels.len());
        for name in channels {
            let column = self
                .channels
                .iter()
                .position(|ch| &ch.name == name)
                .ok_or_else(|| invalid("Unknown waveform channel"))?;
            if columns.contains(&column) {
                return Err(invalid("Duplicate waveform channel"));
            }
            columns.push(column);
            let channel = &self.channels[column];
            series.push(Series {
                name: &channel.name,
                unit: &channel.unit,
                minimum: None,
                maximum: None,
                points: Vec::new(),
            });
        }

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
                if row > visible.start && row + 1 < visible.end {
                    let before = self.row(row - 1)[column];
                    let value = self.row(row)[column];
                    let after = self.row(row + 1)[column];
                    if (value > before && value > after) || (value < before && value < after) {
                        for sample in row - 1..=row + 1 {
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
