//! Streaming gzip reader (RFC 1952) over `miniz_oxide`'s raw inflater, so a
//! large `.gz` file (the 320 MB MPCORB catalogue) can be read line by line
//! without ever being decompressed to disk or held in memory.

use std::io::{self, BufRead, Read};

use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::zip::crc32_update;

const FHCRC: u8 = 2;
const FEXTRA: u8 = 4;
const FNAME: u8 = 8;
const FCOMMENT: u8 = 16;

/// Decompresses one gzip member. The trailer's CRC-32 and length are
/// checked at the end, so a truncated or corrupt file is an error, never a
/// silently short read.
pub struct GzReader<R: BufRead> {
    inner: R,
    state: Box<InflateState>,
    crc: u32,
    size: u32,
    done: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

impl<R: BufRead> GzReader<R> {
    /// Reads and checks the gzip header.
    pub fn new(mut inner: R) -> io::Result<GzReader<R>> {
        let mut head = [0u8; 10];
        inner
            .read_exact(&mut head)
            .map_err(|_| invalid("not a gzip file"))?;
        if head[..3] != [0x1f, 0x8b, 8] {
            return Err(invalid("not a gzip file"));
        }
        let flags = head[3];
        if flags & FEXTRA != 0 {
            let mut len = [0u8; 2];
            inner.read_exact(&mut len)?;
            let len = u16::from_le_bytes(len) as u64;
            io::copy(&mut (&mut inner).take(len), &mut io::sink())?;
        }
        for flag in [FNAME, FCOMMENT] {
            if flags & flag != 0 {
                let mut skipped = Vec::new();
                inner.read_until(0, &mut skipped)?;
            }
        }
        if flags & FHCRC != 0 {
            inner.read_exact(&mut [0u8; 2])?;
        }
        Ok(GzReader {
            inner,
            state: InflateState::new_boxed(DataFormat::Raw),
            crc: 0,
            size: 0,
            done: false,
        })
    }

    fn finish(&mut self) -> io::Result<()> {
        self.done = true;
        let mut trailer = [0u8; 8];
        self.inner
            .read_exact(&mut trailer)
            .map_err(|_| invalid("gzip file is truncated"))?;
        let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
        let size = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
        if crc != self.crc || size != self.size {
            return Err(invalid("gzip file is corrupt (checksum mismatch)"));
        }
        Ok(())
    }
}

impl<R: BufRead> Read for GzReader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while !self.done {
            let input = self.inner.fill_buf()?;
            let eof = input.is_empty();
            let flush = if eof { MZFlush::Finish } else { MZFlush::None };
            let r = inflate(&mut self.state, input, out, flush);
            self.inner.consume(r.bytes_consumed);
            let written = &out[..r.bytes_written];
            self.crc = crc32_update(self.crc, written);
            self.size = self.size.wrapping_add(written.len() as u32);
            match r.status {
                Ok(MZStatus::StreamEnd) => self.finish()?,
                Ok(_) | Err(MZError::Buf) if r.bytes_written > 0 || r.bytes_consumed > 0 => {}
                Ok(_) | Err(MZError::Buf) if eof => {
                    return Err(invalid("gzip file is truncated"));
                }
                Ok(_) => {}
                Err(_) => return Err(invalid("gzip file is corrupt")),
            }
            if r.bytes_written > 0 {
                return Ok(r.bytes_written);
            }
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A gzip file around `body`, with a file name field like `gzip` writes.
    fn gzip(body: &[u8]) -> Vec<u8> {
        let mut out = vec![0x1f, 0x8b, 8, FNAME, 0, 0, 0, 0, 0, 3];
        out.extend(b"MPCORB.DAT\0");
        out.extend(miniz_oxide::deflate::compress_to_vec(body, 6));
        out.extend(crc32_update(0, body).to_le_bytes());
        out.extend((body.len() as u32).to_le_bytes());
        out
    }

    fn body() -> Vec<u8> {
        (0..20_000)
            .flat_map(|i| format!("line {i} of the catalogue\n").into_bytes())
            .collect()
    }

    #[test]
    fn streams_lines() {
        let body = body();
        let gz = gzip(&body);
        // A tiny input buffer exercises refills mid-stream.
        let reader = GzReader::new(io::BufReader::with_capacity(7, &gz[..])).unwrap();
        let lines: Vec<String> = io::BufReader::new(reader)
            .lines()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(lines.len(), 20_000);
        assert_eq!(lines[12_345], "line 12345 of the catalogue");
    }

    #[test]
    fn rejects_truncated_and_corrupt() {
        let body = body();
        let gz = gzip(&body);
        let read_all = |bytes: &[u8]| -> io::Result<Vec<u8>> {
            let mut out = Vec::new();
            GzReader::new(bytes)?.read_to_end(&mut out)?;
            Ok(out)
        };
        assert_eq!(read_all(&gz).unwrap(), body);
        assert!(read_all(&gz[..gz.len() / 2]).is_err());
        assert!(read_all(&gz[..gz.len() - 3]).is_err());
        let mut bad = gz.clone();
        let n = bad.len();
        bad[n - 6] ^= 1; // CRC
        assert!(read_all(&bad).is_err());
        assert!(read_all(b"plain text, not gzip").is_err());
    }
}
