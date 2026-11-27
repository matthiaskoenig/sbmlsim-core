//! Explicit trusted-file adapter. This module is absent from browser builds.
use super::*;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

impl OdeSystem {
    /// Read an explicitly trusted file and its local comp companion files once.
    /// HTTP and other network sources are never fetched.
    pub fn read_path(path: &Path, budget: usize) -> Result<Self, String> {
        const MAX_BYTES: usize = 100 * 1024 * 1024;
        let path = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut pending = vec![path.clone()];
        let mut scheduled = HashSet::from([path.clone()]);
        let mut sources = vec![];
        let mut total = 0usize;
        while let Some(path) = pending.pop() {
            let mut bytes = vec![];
            std::fs::File::open(&path)
                .and_then(|file| {
                    file.take((MAX_BYTES - total + 1) as u64)
                        .read_to_end(&mut bytes)
                })
                .map_err(|e| format!("{}: {e}", path.display()))?;
            total = total
                .checked_add(bytes.len())
                .ok_or("The source set exceeds 100 MB.")?;
            if total > MAX_BYTES {
                return Err("The source set exceeds 100 MB.".into());
            }
            let mut document =
                sbmlsim_reader::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            // Canonical source locations are provided explicitly to the closed-set resolver.
            for list in &mut document.element.extensions {
                if list.package != "comp" || list.name != "listOfExternalModelDefinitions" {
                    continue;
                }
                for emd in &mut list.children {
                    let Some(source) = emd.attributes.iter_mut().find(|a| a.name == "source")
                    else {
                        continue;
                    };
                    let Some(local) = local_path(&source.value) else {
                        continue;
                    };
                    let candidate = path.parent().unwrap_or_else(|| Path::new(".")).join(local);
                    let Ok(target) = candidate.canonicalize() else {
                        continue;
                    };
                    if !target.is_file() {
                        continue;
                    }
                    source.value = format!(
                        "/{}",
                        target
                            .to_string_lossy()
                            .replace('\\', "/")
                            .trim_start_matches('/')
                    );
                    if scheduled.insert(target.clone()) {
                        if scheduled.len() > 1000 {
                            return Err("The source set exceeds 1000 documents.".into());
                        }
                        pending.push(target);
                    }
                }
            }
            sources.push((path.to_string_lossy().replace('\\', "/"), document));
        }
        Self::from_documents(sources, budget)
    }
}

// File URIs may name local files only; network authorities and other schemes are not read.
fn local_path(source: &str) -> Option<PathBuf> {
    let source = if let Some(uri) = source.strip_prefix("file:") {
        if let Some(uri) = uri.strip_prefix("//") {
            let (authority, path) = uri.split_once('/')?;
            if !authority.is_empty() && !authority.eq_ignore_ascii_case("localhost") {
                return None;
            }
            if cfg!(windows) {
                path.to_owned()
            } else {
                format!("/{path}")
            }
        } else {
            uri.to_owned()
        }
    } else {
        if !Path::new(source).is_absolute() && sbmlsim_document::locations::is_remote_source(source)
        {
            return None;
        }
        source.to_owned()
    };
    let decoded = sbmlsim_document::locations::decode_source(&source);
    if decoded.starts_with("//") || decoded.starts_with(r"\\") {
        return None;
    }
    Some(PathBuf::from(decoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_local_sources_are_file_paths() {
        for source in [
            "https://example.org/a.xml",
            "file://server/a.xml",
            "file://localhost.evil/a.xml",
            "//server/a.xml",
            r"\\server\a.xml",
        ] {
            assert!(local_path(source).is_none(), "{source}");
        }
        assert_eq!(
            local_path("other%20model.xml"),
            Some(PathBuf::from("other model.xml"))
        );
        if !cfg!(windows) {
            assert_eq!(
                local_path("file:///tmp/model.xml"),
                Some(PathBuf::from("/tmp/model.xml"))
            );
            assert_eq!(
                local_path("file://localhost/tmp/model.xml"),
                Some(PathBuf::from("/tmp/model.xml"))
            );
        }
    }
}
