//! Minimal zip reader: lists entries and extracts stored or deflated files
//! into memory, checking each file's CRC-32. No ZIP64 (files over 4 GB),
//! encryption or multi-disk archives, none of which image sets use.

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

pub struct Entry {
    /// Path inside the archive, `/`-separated.
    pub name: String,
    method: u16,
    crc: u32,
    compressed: usize,
    size: usize,
    local_offset: usize,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Reads the central directory.
pub fn entries(zip: &[u8]) -> Result<Vec<Entry>, String> {
    let bad = || "not a valid zip file".to_string();
    // The end-of-central-directory record sits in the last 64 KB (it may be
    // followed by a comment of up to 65535 bytes).
    let tail_start = zip.len().saturating_sub(22 + 65_535);
    let eocd = (tail_start..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(zip, i) == Some(EOCD_SIG))
        .ok_or_else(bad)?;
    let count = u16_at(zip, eocd + 10).ok_or_else(bad)? as usize;
    let mut at = u32_at(zip, eocd + 16).ok_or_else(bad)? as usize;

    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(zip, at) != Some(CENTRAL_SIG) {
            return Err(bad());
        }
        let field = |off| u32_at(zip, at + off).ok_or_else(bad);
        let short = |off| u16_at(zip, at + off).map(usize::from).ok_or_else(bad);
        let (compressed, size, local) = (field(20)?, field(24)?, field(42)?);
        if [compressed, size, local].contains(&u32::MAX) {
            return Err("zip files over 4 GB are not supported".into());
        }
        let (name_len, extra_len, comment_len) = (short(28)?, short(30)?, short(32)?);
        let name = zip.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
        out.push(Entry {
            name: String::from_utf8_lossy(name).replace('\\', "/"),
            method: short(10)? as u16,
            crc: field(16)?,
            compressed: compressed as usize,
            size: size as usize,
            local_offset: local as usize,
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// Extracts one entry's contents.
pub fn extract(zip: &[u8], entry: &Entry) -> Result<Vec<u8>, String> {
    let bad = || format!("{}: damaged zip entry", entry.name);
    let at = entry.local_offset;
    if u32_at(zip, at) != Some(LOCAL_SIG) {
        return Err(bad());
    }
    let skip = u16_at(zip, at + 26).ok_or_else(bad)? as usize
        + u16_at(zip, at + 28).ok_or_else(bad)? as usize;
    let start = at + 30 + skip;
    let data = zip.get(start..start + entry.compressed).ok_or_else(bad)?;

    let bytes = match entry.method {
        0 => data.to_vec(),
        8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, entry.size)
            .map_err(|_| bad())?,
        m => {
            return Err(format!(
                "{}: unsupported zip compression (method {m})",
                entry.name
            ));
        }
    };
    if bytes.len() != entry.size || crc32(&bytes) != entry.crc {
        return Err(format!(
            "{}: zip entry is corrupt (checksum mismatch)",
            entry.name
        ));
    }
    Ok(bytes)
}

fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

/// Continues a CRC-32 (IEEE, as in zip and gzip) over more data; start at 0.
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *v = c;
        }
        t
    });
    !data.iter().fold(!crc, |c, &b| {
        table[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// A stored (uncompressed) single-file archive built by hand.
    fn stored_zip(name: &str, body: &[u8]) -> Vec<u8> {
        let crc = crc32(body);
        let mut z = Vec::new();
        let local = |z: &mut Vec<u8>| {
            z.extend(LOCAL_SIG.to_le_bytes());
            z.extend([20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            z.extend(crc.to_le_bytes());
            z.extend((body.len() as u32).to_le_bytes());
            z.extend((body.len() as u32).to_le_bytes());
            z.extend((name.len() as u16).to_le_bytes());
            z.extend(0u16.to_le_bytes());
            z.extend(name.as_bytes());
        };
        local(&mut z);
        z.extend(body);
        let cd = z.len();
        z.extend(CENTRAL_SIG.to_le_bytes());
        z.extend([20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend(crc.to_le_bytes());
        z.extend((body.len() as u32).to_le_bytes());
        z.extend((body.len() as u32).to_le_bytes());
        z.extend((name.len() as u16).to_le_bytes());
        z.extend([0u8; 12]);
        z.extend(0u32.to_le_bytes()); // local header offset
        z.extend(name.as_bytes());
        let cd_len = z.len() - cd;
        z.extend(EOCD_SIG.to_le_bytes());
        z.extend([0, 0, 0, 0, 1, 0, 1, 0]);
        z.extend((cd_len as u32).to_le_bytes());
        z.extend((cd as u32).to_le_bytes());
        z.extend(0u16.to_le_bytes());
        z
    }

    #[test]
    fn reads_stored_entry_and_detects_corruption() {
        let mut z = stored_zip("XY57_p01/a.fits", b"SIMPLE  =  T");
        let list = entries(&z).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "XY57_p01/a.fits");
        assert_eq!(extract(&z, &list[0]).unwrap(), b"SIMPLE  =  T");

        let body_at = 30 + "XY57_p01/a.fits".len();
        z[body_at] ^= 0xff;
        assert!(
            extract(&z, &entries(&z).unwrap()[0])
                .unwrap_err()
                .contains("checksum")
        );
    }

    #[test]
    fn rejects_non_zip() {
        assert!(entries(b"definitely not a zip archive at all").is_err());
    }

    /// Real-data check (absolute path; tests run in the crate folder):
    /// `OPENCERES_TEST_ZIP=$PWD/data/set.zip cargo test -p openceres-desktop -- --ignored`
    #[test]
    #[ignore]
    fn real_image_set_zip() {
        let path = std::env::var("OPENCERES_TEST_ZIP").expect("set OPENCERES_TEST_ZIP");
        let zip = std::fs::read(path).unwrap();
        let fits: Vec<_> = entries(&zip)
            .unwrap()
            .into_iter()
            .filter(|e| e.name.ends_with(".fits"))
            .collect();
        assert!(!fits.is_empty());
        for entry in &fits {
            let image = openceres_core::fits::parse(&extract(&zip, entry).unwrap()).unwrap();
            println!("{} {}x{}", entry.name, image.width, image.height);
        }
    }
}
