//! The PNG of an image element (since 0.34.0): its base64 checked and the size read from its
//! header, without decoding the image, which the browser and resvg do.

/// The largest number of pixels of an image of an image element, 64 Mpx, as the largest image
/// of a figure (`MAX_PIXELS` of its PNG): a pixmap of 256 MiB when the PNG of the figure is drawn.
pub const MAX_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;

/// The eight bytes every PNG starts with.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// The value of a character of the standard base64 alphabet.
fn sextet(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// The bytes of the first `groups` groups of four characters of base64 text which is valid as
/// a whole: the standard alphabet, a length of a multiple of four, padding only at its end.
fn decode_prefix(text: &str, groups: usize) -> Result<Vec<u8>, String> {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return Err("is empty, expected the base64 of a PNG".into());
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(format!(
            "is no base64 of the standard alphabet with its padding: {} characters, not a \
             multiple of 4",
            bytes.len()
        ));
    }
    let pad = bytes.iter().rev().take_while(|c| **c == b'=').count();
    if pad > 2 {
        return Err("is no base64: more than two `=` at its end".into());
    }
    if let Some(k) = bytes[..bytes.len() - pad]
        .iter()
        .position(|c| sextet(*c).is_none())
    {
        return Err(format!(
            "is no base64 of the standard alphabet: character {k} is {:?}",
            char::from(bytes[k])
        ));
    }
    let mut out = Vec::with_capacity(3 * groups);
    for chunk in bytes.chunks(4).take(groups) {
        let v: Vec<u32> = chunk
            .iter()
            .map(|c| u32::from(sextet(*c).unwrap_or(0)))
            .collect();
        let n = (v[0] << 18) | (v[1] << 12) | (v[2] << 6) | v[3];
        out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]);
    }
    Ok(out)
}

/// The standard base64 alphabet.
#[cfg(feature = "png")]
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The base64 of bytes, the standard alphabet with its padding.
#[cfg(feature = "png")]
fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The `png` of an image element (since 0.34.0): the base64 of the PNG of an image of
/// `width` by `height` pixels, its rows from the top, each pixel of three bytes (red, green,
/// blue) or of four with `alpha`. An image without pixels, of other bytes than its pixels or of
/// more than [`MAX_IMAGE_PIXELS`] pixels is refused.
#[cfg(feature = "png")]
pub fn encode_png(width: u32, height: u32, alpha: bool, pixels: &[u8]) -> Result<String, String> {
    let channels: u64 = if alpha { 4 } else { 3 };
    let (w, h) = (u64::from(width), u64::from(height));
    if w == 0 || h == 0 {
        return Err(format!("an image of {width} by {height} pixels has none"));
    }
    if w * h > MAX_IMAGE_PIXELS {
        return Err(format!(
            "an image of {width} by {height} pixels is more than the {MAX_IMAGE_PIXELS} pixels              (64 Mpx) of an image"
        ));
    }
    if pixels.len() as u64 != w * h * channels {
        return Err(format!(
            "{} bytes are no image of {width} by {height} pixels of {channels} bytes",
            pixels.len()
        ));
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(if alpha {
        png::ColorType::Rgba
    } else {
        png::ColorType::Rgb
    });
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(pixels).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(encode_base64(&out))
}

/// The width and the height of the PNG of base64 text: the text checked as base64 and the
/// header of the PNG read, its signature and its first chunk `IHDR`, every side at least 1
/// pixel and at most [`MAX_IMAGE_PIXELS`] pixels in all.
pub(crate) fn png_size(text: &str) -> Result<(u32, u32), String> {
    // the signature and the width and height of `IHDR` are the first 24 bytes
    let head = decode_prefix(text, 8)?;
    if head.len() < 24 || head[..8] != SIGNATURE {
        return Err("is no PNG: its bytes do not start with the signature of a PNG".into());
    }
    if &head[12..16] != b"IHDR" {
        return Err("is no PNG: its first chunk is not IHDR".into());
    }
    let word = |k: usize| u32::from_be_bytes([head[k], head[k + 1], head[k + 2], head[k + 3]]);
    let (w, h) = (word(16), word(20));
    if w == 0 || h == 0 {
        return Err(format!("is an image of {w} by {h} pixels, which has none"));
    }
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
        return Err(format!(
            "is an image of {w} by {h} pixels, more than the {MAX_IMAGE_PIXELS} pixels (64 Mpx) \
             of an image"
        ));
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG of 1 by 1 pixel, white.
    const WHITE: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4//8/AAX+Av4N70a4AAAAAElFTkSuQmCC";

    #[test]
    fn the_size_of_a_png() {
        assert_eq!(png_size(WHITE), Ok((1, 1)));
    }

    #[cfg(feature = "png")]
    #[test]
    fn an_encoded_png_is_read_back() {
        assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
        assert_eq!(encode_base64(b"hell"), "aGVsbA==");
        assert_eq!(encode_base64(b"hel"), "aGVs");
        let pixels: Vec<u8> = (0..2 * 3 * 4).map(|k| (k * 10) as u8).collect();
        let text = encode_png(2, 3, true, &pixels).unwrap();
        assert_eq!(png_size(&text), Ok((2, 3)));
        assert!(
            encode_png(2, 3, false, &pixels)
                .unwrap_err()
                .contains("24 bytes")
        );
        assert!(encode_png(0, 3, false, &[]).is_err());
    }

    #[test]
    fn what_is_no_png() {
        assert!(png_size("").unwrap_err().contains("empty"));
        assert!(png_size("abc").unwrap_err().contains("multiple of 4"));
        assert!(png_size("ab!d").unwrap_err().contains("character 2"));
        assert!(
            png_size("aGVsbG8gd29ybGQgaGVsbG8gd29ybGQh")
                .unwrap_err()
                .contains("signature")
        );
        assert!(png_size("ab=d").unwrap_err().contains("character 2"));
    }
}
