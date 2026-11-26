//! The compilation of the C of a model into a shared library, the cache of the libraries and
//! their loading.
//!
//! The compiler is `$CC` (a command and its leading arguments, `ccache gcc`), else `cc`, with
//! [`FLAGS`]: `-march=native` for the machine, and no contraction into fused multiply adds and
//! no fast math, so every operation of the C is the IEEE operation it writes. A library is
//! `<key>.so` (`.dylib` on macOS) in the cache directory, next to its source `<key>.c`; the key
//! is the SHA-256 of the flags, the first line of `--version` of the compiler, the macros the
//! compiler predefines under the flags (the features of the machine `-march=native` selects, so
//! a cache shared by machines of other processors keeps a library per processor) and the
//! source. A library is compiled into a temporary file and renamed into place, so concurrent
//! builds of one model (the processes of a parallel run) never see a partial one.
//!
//! The cache directory is `$SBMLSIM_C_CACHE`, else `$XDG_CACHE_HOME/sbmlsim/c`, else
//! `~/.cache/sbmlsim/c`. A library a process has loaded is shared by its callers as long as one
//! of them holds it ([`Library::build`] gives the same `Arc`).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use sha2::{Digest, Sha256};

use crate::CError;

/// The flags of the compilation of a library.
pub const FLAGS: [&str; 6] = [
    "-O3",
    "-march=native",
    "-ffp-contract=off",
    "-fno-fast-math",
    "-fPIC",
    "-shared",
];

/// The libraries a library links: the math library of `<math.h>`.
const LIBRARIES: [&str; 1] = ["-lm"];

/// The version of the layout of the cache, in the key.
const CACHE_VERSION: &str = "sbmlsim-c cache 1";

/// Where and with what a library is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildOptions {
    /// The compiler: a command and its leading arguments, separated by whitespace.
    pub compiler: String,
    /// The directory of the cached libraries.
    pub cache_dir: PathBuf,
}

impl BuildOptions {
    /// The compiler `$CC` (else `cc`) and the cache directory `$SBMLSIM_C_CACHE` (else
    /// `$XDG_CACHE_HOME/sbmlsim/c`, else `~/.cache/sbmlsim/c`).
    pub fn from_env() -> Result<BuildOptions, CError> {
        let compiler = compiler_of(std::env::var_os("CC"));
        let cache_dir = cache_dir_of(
            std::env::var_os("SBMLSIM_C_CACHE"),
            std::env::var_os("XDG_CACHE_HOME"),
            std::env::home_dir(),
        )
        .ok_or_else(|| {
            CError::Load(
                "no cache directory of the compiled models: set SBMLSIM_C_CACHE".to_string(),
            )
        })?;
        Ok(BuildOptions {
            compiler,
            cache_dir,
        })
    }
}

/// The compiler of the value of `CC`: `cc` where it is unset or blank.
fn compiler_of(cc: Option<OsString>) -> String {
    cc.map(|c| c.to_string_lossy().trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "cc".to_string())
}

/// The cache directory of the values of `SBMLSIM_C_CACHE`, `XDG_CACHE_HOME` (used only when it
/// is absolute, as the XDG specification says) and the home directory.
fn cache_dir_of(
    cache: Option<OsString>,
    xdg: Option<OsString>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(cache) = cache.filter(|c| !c.is_empty()) {
        return Some(PathBuf::from(cache));
    }
    let base = xdg
        .map(PathBuf::from)
        .filter(|x| x.is_absolute())
        .or_else(|| home.map(|h| h.join(".cache")))?;
    Some(base.join("sbmlsim").join("c"))
}

/// A compiler, as identified for the key of the cache.
struct Compiler {
    /// The command as given (the error of a compiler names it).
    text: String,
    /// The program.
    program: String,
    /// The leading arguments.
    args: Vec<String>,
    /// The first line of its `--version`.
    version: String,
    /// The macros it predefines under [`FLAGS`], sorted.
    macros: String,
}

impl Compiler {
    /// The command of the program with the leading arguments.
    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }

    /// Runs the compiler with `args`; its standard output.
    fn run(&self, args: &[&str]) -> Result<String, CError> {
        let output = self
            .command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| self.spawn_error(e))?;
        if !output.status.success() {
            return Err(CError::Compiler {
                command: self.text.clone(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// The error of a compiler which does not start.
    fn spawn_error(&self, error: std::io::Error) -> CError {
        match error.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => CError::NoCompiler {
                command: self.text.clone(),
            },
            _ => CError::Compiler {
                command: self.text.clone(),
                stderr: error.to_string(),
            },
        }
    }

    /// The compiler of a command, identified once per process.
    fn of(text: &str) -> Result<Arc<Compiler>, CError> {
        static COMPILERS: Mutex<Option<HashMap<String, Arc<Compiler>>>> = Mutex::new(None);
        if let Some(compiler) = COMPILERS.lock().unwrap().get_or_insert_default().get(text) {
            return Ok(compiler.clone());
        }
        let mut words = text.split_whitespace().map(str::to_string);
        let program = words.next().ok_or_else(|| CError::NoCompiler {
            command: text.to_string(),
        })?;
        let mut compiler = Compiler {
            text: text.to_string(),
            program,
            args: words.collect(),
            version: String::new(),
            macros: String::new(),
        };
        compiler.version = compiler
            .run(&["--version"])?
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        let mut probe: Vec<&str> = FLAGS.to_vec();
        probe.extend(["-dM", "-E", "-x", "c", "-"]);
        let mut macros: Vec<String> = compiler.run(&probe)?.lines().map(String::from).collect();
        macros.sort();
        compiler.macros = macros.join("\n");
        let compiler = Arc::new(compiler);
        COMPILERS
            .lock()
            .unwrap()
            .get_or_insert_default()
            .insert(text.to_string(), compiler.clone());
        Ok(compiler)
    }

    /// The key of the library of `source`, hexadecimal.
    fn key(&self, source: &str) -> String {
        let mut parts: Vec<&str> = vec![CACHE_VERSION];
        parts.extend(FLAGS);
        parts.extend(LIBRARIES);
        parts.extend([self.version.as_str(), self.macros.as_str(), source]);
        let mut hash = Sha256::new();
        for part in parts {
            // every part with its length, so no two lists of parts give the same bytes
            hash.update((part.len() as u64).to_le_bytes());
            hash.update(part.as_bytes());
        }
        hash.finalize()
            .iter()
            .fold(String::with_capacity(64), |mut hex, b| {
                use std::fmt::Write as _;
                write!(hex, "{b:02x}").unwrap();
                hex
            })
    }
}

/// A compiled model loaded into the process.
#[derive(Debug)]
pub struct Library {
    library: libloading::Library,
    path: PathBuf,
}

/// The libraries the process has loaded, by key, while a caller holds them.
static LOADED: Mutex<Option<HashMap<String, Weak<Library>>>> = Mutex::new(None);

/// A number for the temporary files of the builds of this process.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

impl Library {
    /// The library of the C `source`, with the compiler and the cache directory of the
    /// environment ([`BuildOptions::from_env`]).
    pub fn build(source: &str) -> Result<Arc<Library>, CError> {
        Library::build_with(source, &BuildOptions::from_env()?)
    }

    /// The library of the C `source`: the one the process has loaded, else the one of the
    /// cache, else compiled into the cache; loaded.
    pub fn build_with(source: &str, options: &BuildOptions) -> Result<Arc<Library>, CError> {
        let compiler = Compiler::of(&options.compiler)?;
        let key = compiler.key(source);
        if let Some(library) = LOADED
            .lock()
            .unwrap()
            .get_or_insert_default()
            .get(&key)
            .and_then(Weak::upgrade)
        {
            return Ok(library);
        }
        let dir = &options.cache_dir;
        let path = dir.join(format!("{key}{}", std::env::consts::DLL_SUFFIX));
        let library = match path.is_file().then(|| Library::load(&path)) {
            Some(Ok(library)) => library,
            // absent, or a file which does not load (compiled again over it)
            _ => {
                compile(&compiler, source, dir, &key, &path)?;
                Library::load(&path)?
            }
        };
        let mut loaded = LOADED.lock().unwrap();
        let loaded = loaded.get_or_insert_default();
        loaded.retain(|_, l| l.strong_count() > 0);
        // a library another thread loaded meanwhile is the one of the process
        if let Some(other) = loaded.get(&key).and_then(Weak::upgrade) {
            return Ok(other);
        }
        let library = Arc::new(library);
        loaded.insert(key, Arc::downgrade(&library));
        Ok(library)
    }

    /// Loads the library at `path`.
    fn load(path: &Path) -> Result<Library, CError> {
        // SAFETY: the library is C of `emit.rs` compiled with the flags above: it has no
        // initializer or finalizer which could run on load or unload.
        let library = unsafe { libloading::Library::new(path) }
            .map_err(|e| CError::Load(format!("{}: {e}", path.display())))?;
        Ok(Library {
            library,
            path: path.to_path_buf(),
        })
    }

    /// The file of the library.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The function or the value of the symbol `name`.
    ///
    /// # Safety
    ///
    /// `T` must be the type of the symbol (an `unsafe extern "C" fn` of the signature the C
    /// defines), and the value must not be used after the library is dropped.
    pub unsafe fn symbol<T: Copy>(&self, name: &str) -> Result<T, CError> {
        // SAFETY: the caller guarantees the type and the lifetime.
        unsafe { self.library.get::<T>(name.as_bytes()) }
            .map(|symbol| *symbol)
            .map_err(|e| CError::Load(format!("{name} of {}: {e}", self.path.display())))
    }
}

/// Compiles `source` into the library `path` of the cache directory `dir`.
fn compile(
    compiler: &Compiler,
    source: &str,
    dir: &Path,
    key: &str,
    path: &Path,
) -> Result<(), CError> {
    let io = |what: &str, p: &Path, e: std::io::Error| {
        CError::Load(format!("cannot {what} {}: {e}", p.display()))
    };
    fs::create_dir_all(dir).map_err(|e| io("create the cache directory", dir, e))?;
    let unique = format!(
        "{}.{}",
        std::process::id(),
        TEMPORARY.fetch_add(1, Ordering::Relaxed)
    );
    // the source, renamed into place, then the library
    let c = dir.join(format!("{key}.c"));
    let c_temporary = dir.join(format!(".{key}.{unique}.c"));
    let mut file = fs::File::create(&c_temporary).map_err(|e| io("write", &c_temporary, e))?;
    file.write_all(source.as_bytes())
        .map_err(|e| io("write", &c_temporary, e))?;
    drop(file);
    fs::rename(&c_temporary, &c).map_err(|e| io("write", &c, e))?;

    let temporary = dir.join(format!(".{key}.{unique}{}", std::env::consts::DLL_SUFFIX));
    let output = compiler
        .command()
        .args(FLAGS)
        .arg("-o")
        .arg(&temporary)
        .arg(&c)
        .args(LIBRARIES)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| compiler.spawn_error(e))?;
    if !output.status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(CError::Compiler {
            command: compiler.text.clone(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    fs::rename(&temporary, path).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        io("write", path, e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compiler_is_cc_unless_cc_names_one() {
        assert_eq!(compiler_of(None), "cc");
        assert_eq!(compiler_of(Some(" ".into())), "cc");
        assert_eq!(compiler_of(Some("ccache gcc".into())), "ccache gcc");
    }

    #[test]
    fn the_cache_directory_follows_the_environment() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            cache_dir_of(Some("/c".into()), Some("/x".into()), home.clone()),
            Some(PathBuf::from("/c"))
        );
        assert_eq!(
            cache_dir_of(None, Some("/x".into()), home.clone()),
            Some(PathBuf::from("/x/sbmlsim/c"))
        );
        // a relative XDG_CACHE_HOME is ignored, as the specification says
        assert_eq!(
            cache_dir_of(Some("".into()), Some("x".into()), home.clone()),
            Some(PathBuf::from("/home/u/.cache/sbmlsim/c"))
        );
        assert_eq!(cache_dir_of(None, None, None), None);
    }
}
