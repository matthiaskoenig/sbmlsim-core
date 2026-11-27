//! Decode XML's mandatory encodings and the legacy Latin-1 SBML documents.
use crate::ReadError;
use std::borrow::Cow;

pub(crate) fn decode(source: &[u8]) -> Result<Cow<'_, str>, ReadError> {
    let little = source.starts_with(&[0xff, 0xfe]) || source.starts_with(&[b'<', 0, b'?', 0]);
    let big = source.starts_with(&[0xfe, 0xff]) || source.starts_with(&[0, b'<', 0, b'?']);
    if little || big {
        let bytes = if source.starts_with(&[0xff, 0xfe]) || source.starts_with(&[0xfe, 0xff]) {
            &source[2..]
        } else {
            source
        };
        if bytes.len() % 2 != 0 {
            return Err(ReadError("Truncated UTF-16 XML code unit.".into()));
        }
        let units = bytes.as_chunks::<2>().0.iter().map(|c| {
            if little {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        });
        let text = char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|e| ReadError(format!("Invalid UTF-16 XML: {e}")))?;
        return Ok(Cow::Owned(text));
    }
    let header = &source[..source.len().min(512)];
    let header = String::from_utf8_lossy(header);
    let header = header
        .split_once("?>")
        .map(|(p, _)| p)
        .unwrap_or("")
        .to_ascii_lowercase();
    let encoding = header
        .split_once("encoding")
        .and_then(|(_, s)| s.trim_start().strip_prefix('='))
        .map(str::trim_start)
        .and_then(|s| {
            let quote = s.chars().next()?;
            if !matches!(quote, '\'' | '"') {
                return None;
            }
            s[1..].split(quote).next()
        });
    match encoding {
        Some("iso-8859-1" | "iso_8859-1" | "latin1" | "latin-1") => {
            Ok(Cow::Owned(source.iter().map(|b| char::from(*b)).collect()))
        }
        Some("windows-1252" | "cp1252") => {
            const C1: [char; 32] = [
                '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}',
                '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8d}',
                '\u{17d}', '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}',
                '\u{2022}', '\u{2013}', '\u{2014}', '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}',
                '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
            ];
            Ok(Cow::Owned(
                source
                    .iter()
                    .map(|b| {
                        if (0x80..=0x9f).contains(b) {
                            C1[(b - 0x80) as usize]
                        } else {
                            char::from(*b)
                        }
                    })
                    .collect(),
            ))
        }
        Some(encoding) if !matches!(encoding, "utf-8" | "utf8" | "us-ascii" | "ascii") => {
            Err(ReadError(format!("Unsupported XML encoding '{encoding}'.")))
        }
        _ => std::str::from_utf8(source)
            .map(Cow::Borrowed)
            .map_err(|e| ReadError(format!("The SBML source is not UTF-8: {e}"))),
    }
}
