//! Persistent compilation caching, managed and versioned by Wasmtime.

use std::path::PathBuf;

use wasmtime::{Cache, CacheConfig};

/// Where compiled Wasmtime modules are cached. This caches machine code, not model state
/// or simulation results. Use a directory writable only by trusted users.
#[derive(Debug, Clone, Default)]
pub enum ModuleCache {
    /// Wasmtime's platform cache directory, independent of its CLI configuration file.
    #[default]
    Default,
    /// Compile without reading or writing the persistent cache.
    Disabled,
    /// A custom cache directory. Relative paths are resolved from the current directory.
    Directory(PathBuf),
}

impl ModuleCache {
    /// Caching is optional: unavailable directories must not prevent a runtime starting.
    pub(crate) fn open(self) -> Option<Cache> {
        let mut config = CacheConfig::new();
        match self {
            Self::Disabled => return None,
            Self::Default => {}
            Self::Directory(path) => {
                let path = if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir().ok()?.join(path)
                };
                config.with_directory(path);
            }
        }
        // Wasmtime also treats missing, unreadable or corrupt entries as cache misses,
        // and writes replacements atomically. Keep those semantics rather than loading
        // serialized machine code ourselves.
        Cache::new(config).ok()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use sbmlsim_abi::{ModelDescription, SimulationRequest};
    use tempfile::TempDir;

    use super::*;
    use crate::Runtime;

    fn model() -> (Vec<u8>, ModelDescription) {
        (
            wat::parse_str(include_str!("../tests/decay.wat")).unwrap(),
            serde_json::from_str(include_str!("../tests/decay-model/model.json")).unwrap(),
        )
    }

    fn simulate(runtime: &Runtime) -> Vec<u64> {
        let (wasm, desc) = model();
        let request: SimulationRequest =
            serde_json::from_str(include_str!("../tests/decay-model/request.json")).unwrap();
        runtime
            .instantiate(&wasm, &desc)
            .unwrap()
            .simulate(&request)
            .unwrap()
            .values
            .iter()
            .map(|value| value.to_bits())
            .collect()
    }

    #[test]
    fn modules_are_cached_across_processes() {
        const DIRECTORY: &str = "SBMLSIM_CACHE_TEST_DIRECTORY";
        const WARM: &str = "SBMLSIM_CACHE_TEST_WARM";
        if let Some(path) = std::env::var_os(DIRECTORY) {
            let runtime =
                Runtime::embedded_with_cache(ModuleCache::Directory(path.into())).unwrap();
            let cache = runtime.cache.as_ref().unwrap();
            let warm = std::env::var_os(WARM).is_some();
            assert_eq!(cache.cache_hits(), if warm { 2 } else { 0 });
            assert_eq!(cache.cache_misses(), if warm { 0 } else { 2 });
            let actual = simulate(&runtime);
            assert_eq!(cache.cache_hits(), if warm { 3 } else { 0 });
            assert_eq!(cache.cache_misses(), if warm { 0 } else { 3 });

            let uncached = Runtime::embedded_with_cache(ModuleCache::Disabled).unwrap();
            assert_eq!(actual, simulate(&uncached));
            assert!(uncached.cache_directory().is_none());

            // A different binary must miss even when it computes exactly the same values.
            let (mut wasm, desc) = model();
            // Use a different custom section in each child so neither variant is warm.
            wasm.extend_from_slice(&[0, 2, 1, if warm { b'y' } else { b'x' }]);
            runtime.load(&wasm, &desc).unwrap();
            assert_eq!(cache.cache_misses(), if warm { 1 } else { 4 });

            // Description and module validation are still applied on cache hits.
            let (wasm, mut desc) = model();
            desc.contract = "invalid".into();
            assert!(runtime.load(&wasm, &desc).is_err());
            return;
        }
        let directory = TempDir::new().unwrap();
        for warm in [false, true] {
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "cache::tests::modules_are_cached_across_processes",
                ])
                .env(DIRECTORY, directory.path())
                .env_remove(WARM);
            if warm {
                command.env(WARM, "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "warm={warm}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn unavailable_cache_does_not_prevent_simulation() {
        let directory = TempDir::new().unwrap();
        let blocked = directory.path().join("a-file");
        fs::write(&blocked, b"not a directory").unwrap();
        let runtime =
            Runtime::embedded_with_cache(ModuleCache::Directory(blocked.join("cache"))).unwrap();
        assert!(runtime.cache_directory().is_none());
        let uncached = Runtime::embedded_with_cache(ModuleCache::Disabled).unwrap();
        assert_eq!(simulate(&runtime), simulate(&uncached));

        // Failure to write individual entries is also harmless, even if the cache root
        // itself is available when the runtime starts.
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join("modules"), b"not a directory").unwrap();
        let runtime =
            Runtime::embedded_with_cache(ModuleCache::Directory(directory.path().to_path_buf()))
                .unwrap();
        assert!(runtime.cache_directory().is_some());
        assert_eq!(simulate(&runtime), simulate(&uncached));
    }

    fn corrupt_entries(directory: &Path) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                count += corrupt_entries(&entry.path());
            } else if entry.path().extension().is_none() {
                fs::write(entry.path(), b"not a compressed cache entry").unwrap();
                count += 1;
            }
        }
        count
    }

    #[test]
    fn corrupt_cache_entries_are_recompiled() {
        let directory = TempDir::new().unwrap();
        let open = || {
            Runtime::embedded_with_cache(ModuleCache::Directory(directory.path().to_path_buf()))
                .unwrap()
        };
        let expected = simulate(&open());
        assert_eq!(corrupt_entries(&directory.path().join("modules")), 3);
        let runtime = open();
        assert_eq!(simulate(&runtime), expected);
        let cache = runtime.cache.as_ref().unwrap();
        assert_eq!(cache.cache_hits(), 0);
        assert_eq!(cache.cache_misses(), 3);
        drop(runtime);
        let runtime = open();
        assert_eq!(simulate(&runtime), expected);
        let cache = runtime.cache.as_ref().unwrap();
        assert_eq!(cache.cache_hits(), 3);
        assert_eq!(cache.cache_misses(), 0);
    }

    #[test]
    fn relative_cache_directories_are_resolved_without_changing_cwd() {
        let directory = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let relative = directory
            .path()
            .strip_prefix(std::env::current_dir().unwrap())
            .unwrap();
        let cache = ModuleCache::Directory(relative.to_path_buf())
            .open()
            .unwrap();
        assert_eq!(cache.directory(), &directory.path().canonicalize().unwrap());
    }
}
