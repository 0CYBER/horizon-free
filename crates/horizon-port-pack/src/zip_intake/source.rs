use std::io::{self, Read, Seek, SeekFrom};

/// Reads an archive prefix from its original source and its suffix from a snapshot.
#[derive(Debug)]
pub struct Source<R> {
    reader: R,
    start: u64,
    bytes: Vec<u8>,
    position: u64,
}

impl<R> Source<R> {
    /// Starts at the frozen suffix. Seeking into the prefix positions the underlying reader.
    pub(super) fn new(reader: R, start: u64, bytes: Vec<u8>) -> Self {
        Self {
            reader,
            start,
            bytes,
            position: start,
        }
    }
}

impl<R: Read> Read for Source<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = if self.position < self.start {
            let remaining = usize::try_from(self.start - self.position).unwrap_or(usize::MAX);
            let limit = buf.len().min(remaining);
            self.reader.read(&mut buf[..limit])?
        } else {
            let offset = usize::try_from(self.position - self.start).unwrap_or(usize::MAX);
            let suffix = self.bytes.get(offset..).unwrap_or_default();
            let count = buf.len().min(suffix.len());
            buf[..count].copy_from_slice(&suffix[..count]);
            count
        };
        self.position = self
            .position
            .checked_add(count as u64)
            .ok_or_else(invalid_position)?;
        Ok(count)
    }
}

impl<R: Seek> Seek for Source<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(offset) => self.position.checked_add_signed(offset),
            SeekFrom::End(offset) => self
                .start
                .checked_add(self.bytes.len() as u64)
                .and_then(|end| end.checked_add_signed(offset)),
        }
        .ok_or_else(invalid_position)?;

        if position < self.start {
            self.reader.seek(SeekFrom::Start(position))?;
        }
        self.position = position;
        Ok(position)
    }
}

fn invalid_position() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "ZIP position is out of range")
}
