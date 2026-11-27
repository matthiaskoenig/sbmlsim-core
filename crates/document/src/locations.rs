//! Closed-set source location handling shared by equation and report adapters.
/// Normalize an external source path within the archive root.
pub fn normalize_location(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}
pub fn is_remote_source(source: &str) -> bool {
    let Some((scheme, _)) = source.split_once(':') else {
        return false;
    };
    scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}
pub fn decode_source(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(a), Some(b)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            output.push((16 * a + b) as u8);
            i += 3;
        } else {
            output.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}
/// Resolve an external source URI against its referring manifest location.
pub fn resolve_source(location: &str, source: &str) -> Option<String> {
    if source.is_empty() || is_remote_source(source) {
        return None;
    }
    let source = decode_source(source);
    if source.starts_with('/') {
        return normalize_location(&source);
    }
    let base = normalize_location(location)?;
    let directory = base.rsplit_once('/').map_or("", |(directory, _)| directory);
    normalize_location(&format!("{directory}/{source}"))
}
