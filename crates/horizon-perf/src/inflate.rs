use flate2::{Crc, Decompress, FlushDecompress, Status};

use crate::error::Error;

const OUT_CHUNK: usize = 64 * 1024;
/// The longest gzip header accepted. The capture writer's header is 10 bytes; the optional extra
/// field, name and comment of a longer one are scanned again for each slice that arrives, so this
/// cap also bounds that work.
const MAX_HEADER: usize = 64 * 1024;

enum State {
    Header(Vec<u8>),
    Body(Box<Decompress>),
    Trailer(Vec<u8>),
    Done,
}

/// Inflates one gzip member from bytes that arrive in any split, appending whatever is whole so far.
pub struct GzipInflater {
    state: State,
    crc: Crc,
    produced: u64,
    limit: u64,
    /// Where one inflate step writes before its bytes are counted and kept.
    scratch: Vec<u8>,
}

impl Default for GzipInflater {
    fn default() -> Self {
        Self {
            state: State::Header(Vec::new()),
            crc: Crc::new(),
            produced: 0,
            limit: crate::follow::MAX_INFLATED,
            scratch: Vec::new(),
        }
    }
}

impl GzipInflater {
    /// An inflater that fails once the stream has produced more than `limit` bytes.
    pub fn with_limit(limit: u64) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    /// True once the trailer has arrived and matched.
    pub fn is_done(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Feeds `bytes` and appends the newly inflated bytes to `out`.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<u8>) -> Result<(), Error> {
        match &mut self.state {
            State::Header(buf) => {
                buf.extend_from_slice(bytes);
                check_magic(buf)?;
                let len = header_len(buf);
                if len.unwrap_or(buf.len()) > MAX_HEADER {
                    return Err(Error::Corrupt(format!(
                        "the gzip header is longer than {} KiB",
                        MAX_HEADER >> 10
                    )));
                }
                let Some(len) = len else {
                    return Ok(());
                };
                let rest = buf.split_off(len);
                self.state = State::Body(Box::new(Decompress::new(false)));
                self.push(&rest, out)
            }
            State::Body(inflater) => {
                let start = out.len();
                let mut input = bytes;
                let mut ended = false;
                // One step writes into a scratch slice no longer than what the limit leaves, plus
                // one byte to see it passed, and only then into `out`.
                let scratch = &mut self.scratch;
                scratch.resize(OUT_CHUNK, 0);
                loop {
                    let room = (self.limit.saturating_sub(self.produced))
                        .saturating_add(1)
                        .min(OUT_CHUNK as u64) as usize;
                    let (before_in, before_out) = (inflater.total_in(), inflater.total_out());
                    let status = inflater
                        .decompress(input, &mut scratch[..room], FlushDecompress::None)
                        .map_err(|e| Error::Corrupt(e.to_string()))?;
                    let made = (inflater.total_out() - before_out) as usize;
                    self.produced += made as u64;
                    if self.produced > self.limit {
                        return Err(Error::Corrupt(format!(
                            "the capture inflates to more than {} MiB",
                            self.limit >> 20
                        )));
                    }
                    out.extend_from_slice(&scratch[..made]);
                    let used = (inflater.total_in() - before_in) as usize;
                    input = &input[used..];
                    if status == Status::StreamEnd {
                        ended = true;
                        break;
                    }
                    if used == 0 && inflater.total_out() == before_out {
                        break;
                    }
                }
                self.crc.update(&out[start..]);
                if ended {
                    let rest = input.to_vec();
                    self.state = State::Trailer(Vec::new());
                    return self.push(&rest, out);
                }
                Ok(())
            }
            State::Trailer(buf) => {
                buf.extend_from_slice(bytes);
                if buf.len() < 8 {
                    return Ok(());
                }
                if buf.len() > 8 {
                    return Err(Error::Corrupt(
                        "data after the end of the gzip stream".into(),
                    ));
                }
                let crc = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                let size = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
                if crc != self.crc.sum() || size != self.crc.amount() {
                    return Err(Error::Corrupt("the gzip checksum does not match".into()));
                }
                self.state = State::Done;
                Ok(())
            }
            State::Done if bytes.is_empty() => Ok(()),
            State::Done => Err(Error::Corrupt(
                "data after the end of the gzip stream".into(),
            )),
        }
    }
}

fn check_magic(buf: &[u8]) -> Result<(), Error> {
    for (i, want) in [0x1f, 0x8b, 8].into_iter().enumerate() {
        if buf.get(i).is_some_and(|&b| b != want) {
            return Err(Error::NotGzip);
        }
    }
    Ok(())
}

/// Length of the RFC 1952 header, or `None` while it is incomplete.
fn header_len(buf: &[u8]) -> Option<usize> {
    if buf.len() < 10 {
        return None;
    }
    let flags = buf[3];
    let mut at = 10;
    if flags & 0x04 != 0 {
        let len = u16::from_le_bytes([*buf.get(at)?, *buf.get(at + 1)?]) as usize;
        at += 2 + len;
    }
    for bit in [0x08, 0x10] {
        if flags & bit != 0 {
            at += buf.get(at..)?.iter().position(|&b| b == 0)? + 1;
        }
    }
    if flags & 0x02 != 0 {
        at += 2;
    }
    (buf.len() >= at).then_some(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_more_than_the_limit_is_kept_and_a_stream_ending_at_it_is_whole() {
        use std::io::Write;
        let gzip = |len: usize| {
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            gz.write_all(&vec![0u8; len]).unwrap();
            gz.finish().unwrap()
        };
        let mut out = Vec::new();
        let mut past = GzipInflater::with_limit(1000);
        assert!(past.push(&gzip(1 << 20), &mut out).is_err());
        assert!(out.len() <= 1000, "{}", out.len());
        let mut out = Vec::new();
        let mut exact = GzipInflater::with_limit(1000);
        exact.push(&gzip(1000), &mut out).unwrap();
        assert!(exact.is_done());
        assert_eq!(out.len(), 1000);
    }

    #[test]
    fn a_gzip_header_past_the_cap_is_refused_while_it_arrives() {
        // A header announcing a file name (FNAME) that never ends.
        let mut inflater = GzipInflater::default();
        let mut out = Vec::new();
        inflater
            .push(&[0x1f, 0x8b, 8, 0x08, 0, 0, 0, 0, 0, 0xff], &mut out)
            .unwrap();
        let slice = [b'a'; 1024];
        let refused = (0..=MAX_HEADER / slice.len())
            .map(|_| inflater.push(&slice, &mut out))
            .any(|r| r.is_err());
        assert!(refused);
    }
}
