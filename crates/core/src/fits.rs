//! Minimal FITS reader for 2-D images in the primary HDU.
//!
//! Supports every standard BITPIX (8, 16, 32, 64, -32, -64) with BZERO/BSCALE
//! applied. Pixel values are returned as `f32`, row by row in file order.

use std::fmt;
use std::path::Path;

const BLOCK: usize = 2880;
const CARD: usize = 80;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Logical(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

#[derive(Debug, Clone, Default)]
pub struct Header {
    cards: Vec<(String, Value)>,
}

impl Header {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.cards.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn float(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn int(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &(String, Value)> {
        self.cards.iter()
    }
}

#[derive(Debug, Clone)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
    pub header: Header,
}

impl Image {
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.width + x]
    }
}

#[derive(Debug)]
pub enum FitsError {
    Io(std::io::Error),
    Format(String),
}

impl fmt::Display for FitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FitsError::Io(e) => write!(f, "cannot read file: {e}"),
            FitsError::Format(m) => write!(f, "invalid FITS file: {m}"),
        }
    }
}

impl std::error::Error for FitsError {}

impl From<std::io::Error> for FitsError {
    fn from(e: std::io::Error) -> Self {
        FitsError::Io(e)
    }
}

fn format_err(msg: impl Into<String>) -> FitsError {
    FitsError::Format(msg.into())
}

pub fn read(path: impl AsRef<Path>) -> Result<Image, FitsError> {
    parse(&std::fs::read(path)?)
}

pub fn parse(bytes: &[u8]) -> Result<Image, FitsError> {
    let (header, header_len) = parse_header(bytes)?;
    if header.get("SIMPLE") != Some(&Value::Logical(true)) {
        return Err(format_err("missing SIMPLE = T"));
    }

    let naxis = header
        .int("NAXIS")
        .ok_or_else(|| format_err("missing NAXIS"))?;
    let axis = |n: i64| {
        header
            .int(&format!("NAXIS{n}"))
            .filter(|v| *v > 0)
            .map(|v| v as usize)
            .ok_or_else(|| format_err(format!("missing or invalid NAXIS{n}")))
    };
    let (width, height) = (axis(1)?, axis(2)?);
    // Accept degenerate extra axes (e.g. NAXIS3 = 1), nothing else.
    for n in 3..=naxis {
        if axis(n)? != 1 {
            return Err(format_err("only 2-D images are supported"));
        }
    }
    if naxis < 2 {
        return Err(format_err("primary HDU is not an image"));
    }

    let bitpix = header
        .int("BITPIX")
        .ok_or_else(|| format_err("missing BITPIX"))?;
    let bzero = header.float("BZERO").unwrap_or(0.0);
    let bscale = header.float("BSCALE").unwrap_or(1.0);
    let bytes_per_pixel = match bitpix {
        8 => 1,
        16 => 2,
        32 | -32 => 4,
        64 | -64 => 8,
        _ => return Err(format_err(format!("unsupported BITPIX {bitpix}"))),
    };

    let count = width * height;
    let raw = bytes
        .get(header_len..header_len + count * bytes_per_pixel)
        .ok_or_else(|| format_err("file is shorter than its image size"))?;

    let scale = |v: f64| (bzero + bscale * v) as f32;
    let data: Vec<f32> = match bitpix {
        8 => raw.iter().map(|&b| scale(b as f64)).collect(),
        16 => raw
            .chunks_exact(2)
            .map(|c| scale(i16::from_be_bytes([c[0], c[1]]) as f64))
            .collect(),
        32 => raw
            .chunks_exact(4)
            .map(|c| scale(i32::from_be_bytes(c.try_into().unwrap()) as f64))
            .collect(),
        64 => raw
            .chunks_exact(8)
            .map(|c| scale(i64::from_be_bytes(c.try_into().unwrap()) as f64))
            .collect(),
        -32 => raw
            .chunks_exact(4)
            .map(|c| scale(f32::from_be_bytes(c.try_into().unwrap()) as f64))
            .collect(),
        _ => raw
            .chunks_exact(8)
            .map(|c| scale(f64::from_be_bytes(c.try_into().unwrap())))
            .collect(),
    };

    Ok(Image {
        width,
        height,
        data,
        header,
    })
}

/// Returns the header and its length in bytes (a whole number of blocks).
fn parse_header(bytes: &[u8]) -> Result<(Header, usize), FitsError> {
    let mut cards = Vec::new();
    for (i, card) in bytes.chunks(CARD).enumerate() {
        if card.len() < CARD {
            break;
        }
        let card = std::str::from_utf8(card).map_err(|_| format_err("header is not ASCII"))?;
        let key = card[..8].trim_end();
        if key == "END" {
            let len = (i + 1) * CARD;
            return Ok((Header { cards }, len.div_ceil(BLOCK) * BLOCK));
        }
        if &card[8..10] == "= " {
            if let Some(value) = parse_value(&card[10..]) {
                cards.push((key.to_string(), value));
            }
        }
    }
    Err(format_err("header has no END card"))
}

fn parse_value(field: &str) -> Option<Value> {
    let field = field.trim_start();
    if let Some(rest) = field.strip_prefix('\'') {
        // Quoted string; '' is an escaped quote.
        let mut text = String::new();
        let mut chars = rest.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    text.push('\'');
                } else {
                    return Some(Value::Text(text.trim_end().to_string()));
                }
            } else {
                text.push(c);
            }
        }
        return None;
    }

    let value = field.split('/').next()?.trim();
    match value {
        "T" => Some(Value::Logical(true)),
        "F" => Some(Value::Logical(false)),
        "" => None,
        _ => value
            .parse::<i64>()
            .map(Value::Int)
            .or_else(|_| {
                value
                    .replace(['D', 'd'], "E")
                    .parse::<f64>()
                    .map(Value::Float)
            })
            .ok(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a FITS file in memory: header cards, then big-endian BITPIX=-32 data.
    pub(crate) fn make_fits(width: usize, height: usize, extra: &[&str], data: &[f32]) -> Vec<u8> {
        let mut cards = vec![
            "SIMPLE  =                    T".to_string(),
            "BITPIX  =                  -32".to_string(),
            "NAXIS   =                    2".to_string(),
            format!("NAXIS1  = {width:>20}"),
            format!("NAXIS2  = {height:>20}"),
        ];
        cards.extend(extra.iter().map(|s| s.to_string()));
        cards.push("END".to_string());
        let mut out = Vec::new();
        for c in &cards {
            out.extend(format!("{c:<80}").as_bytes());
        }
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, b' ');
        for v in data {
            out.extend(v.to_be_bytes());
        }
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
        out
    }

    #[test]
    fn reads_float_image_and_header() {
        let data: Vec<f32> = (0..12).map(|v| v as f32).collect();
        let bytes = make_fits(
            4,
            3,
            &[
                "OBJECT  = 'IASC field 3'       / quoted",
                "EXPTIME =                 45.5",
                "CDELT1  =           -7.0D-05",
                "NOTE    = 'it''s'",
            ],
            &data,
        );
        let img = parse(&bytes).unwrap();
        assert_eq!((img.width, img.height), (4, 3));
        assert_eq!(img.at(1, 2), 9.0);
        assert_eq!(img.header.text("OBJECT"), Some("IASC field 3"));
        assert_eq!(img.header.float("EXPTIME"), Some(45.5));
        assert_eq!(img.header.float("CDELT1"), Some(-7.0e-5));
        assert_eq!(img.header.text("NOTE"), Some("it's"));
    }

    #[test]
    fn applies_bzero_for_unsigned_16_bit() {
        let mut bytes = Vec::new();
        for c in [
            "SIMPLE  =                    T",
            "BITPIX  =                   16",
            "NAXIS   =                    2",
            "NAXIS1  =                    2",
            "NAXIS2  =                    1",
            "BZERO   =                32768",
            "END",
        ] {
            bytes.extend(format!("{c:<80}").as_bytes());
        }
        bytes.resize(BLOCK, b' ');
        bytes.extend((-32768i16).to_be_bytes()); // 0 after BZERO
        bytes.extend(32767i16.to_be_bytes()); // 65535 after BZERO
        bytes.resize(2 * BLOCK, 0);
        let img = parse(&bytes).unwrap();
        assert_eq!(img.data, vec![0.0, 65535.0]);
    }

    #[test]
    fn rejects_truncated_file() {
        let mut bytes = make_fits(4, 4, &[], &[0.0; 16]);
        bytes.truncate(BLOCK + 10);
        assert!(matches!(parse(&bytes), Err(FitsError::Format(_))));
    }
}
