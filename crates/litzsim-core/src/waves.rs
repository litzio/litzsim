use crate::circuit::StagedFile;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
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

/// Publish a complete result at a new path. An existing file is preserved.
pub fn write(path: &Path, wave: &Waveform) -> io::Result<()> {
    let channels = serde_json::to_vec(wave.channels())?;
    let length = u32::try_from(channels.len()).map_err(|_| invalid("channel header too large"))?;
    let rows = wave.rows().len() as u64;
    let staged = StagedFile::write(path, |file| {
        let mut out = BufWriter::new(file);
        out.write_all(MAGIC)?;
        out.write_all(&rows.to_le_bytes())?;
        out.write_all(&length.to_le_bytes())?;
        out.write_all(&channels)?;
        for sample in &wave.samples {
            out.write_all(&sample.to_le_bytes())?;
        }
        out.flush()
    })?;
    fs::hard_link(&staged.0, path)
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
