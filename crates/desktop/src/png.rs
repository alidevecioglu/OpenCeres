//! Minimal PNG writing (one IDAT chunk, no filtering) for screenshots and
//! the cutouts in detailed reports, and base64 for embedding them in JSON as
//! `data:` URLs that any browser can show.

/// 8-bit RGBA, `width × height × 4` bytes, top row first.
pub fn encode_rgba(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    encode(width, height, 4, 6, rgba)
}

/// 8-bit grayscale, `width × height` bytes, top row first.
pub fn encode_gray(width: usize, height: usize, gray: &[u8]) -> Vec<u8> {
    encode(width, height, 1, 0, gray)
}

fn encode(width: usize, height: usize, channels: usize, color_type: u8, data: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width * channels + 1) * height);
    for row in data.chunks_exact(width * channels).take(height) {
        raw.push(0); // filter type: none
        raw.extend_from_slice(row);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    // 8-bit samples, deflate, adaptive filtering, no interlace.
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(
        &mut out,
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6),
    );
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// `data:image/png;base64,…` for a PNG file's bytes.
pub fn data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64(png))
}

/// Standard base64 with padding (RFC 4648).
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let b = [
            group[0],
            *group.get(1).unwrap_or(&0),
            *group.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= group.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_structure() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        let png = encode_rgba(2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[idat - 4..idat].try_into().unwrap()) as usize;
        let raw =
            miniz_oxide::inflate::decompress_to_vec_zlib(&png[idat + 4..idat + 4 + len]).unwrap();
        assert_eq!(raw, [0, 255, 0, 0, 255, 0, 0, 255, 255]);
        // Gray: colour type 0, one byte per pixel.
        let png = encode_gray(2, 1, &[7, 9]);
        assert_eq!(png[25], 0);
    }

    #[test]
    fn base64_matches_rfc_4648() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded);
        }
    }
}
