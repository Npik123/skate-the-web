//! File access seam. Native builds forward to `std::fs` unchanged. The wasm
//! build has no filesystem, so the host mounts files into an in-memory tree
//! before the engine reads them; writes stay in memory and are reported to an
//! optional hook so the page can persist them to browser storage.
use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::*;

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        std::fs::read_to_string(path)
    }
    pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        std::fs::write(path, contents)
    }
    pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }
    pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        std::fs::remove_file(path)
    }
    pub fn is_file(path: impl AsRef<Path>) -> bool {
        path.as_ref().is_file()
    }
    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        path.as_ref().is_dir()
    }
    pub fn exists(path: impl AsRef<Path>) -> bool {
        path.as_ref().exists()
    }
    pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
        path.as_ref().canonicalize()
    }
    /// Native files stay on disk; this is a plain read.
    pub fn take(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }
    /// Native files are always on disk.
    pub fn needs_fetch(_path: impl AsRef<Path>) -> bool {
        false
    }
    pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<DirEntry>> {
        std::fs::read_dir(path)?
            .map(|entry| {
                let entry = entry?;
                Ok(DirEntry { path: entry.path(), is_dir: entry.file_type()?.is_dir() })
            })
            .collect()
    }
}

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::{Arc, Mutex},
    };

    #[derive(Default)]
    struct Tree {
        files: BTreeMap<String, Arc<Vec<u8>>>,
        dirs: BTreeSet<String>,
        /// Files the host can supply on request (maps): listed and reported
        /// as files, but `read` fails until the engine fetches and mounts them.
        declared: BTreeSet<String>,
    }
    static TREE: Mutex<Tree> =
        Mutex::new(Tree { files: BTreeMap::new(), dirs: BTreeSet::new(), declared: BTreeSet::new() });
    static HOOK: Mutex<Option<fn(&str, Option<&[u8]>)>> = Mutex::new(None);
    /// Read tracing (off unless asked for): which mounted files the engine
    /// actually opens, to find data that is downloaded and held for nothing.
    static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static READS: Mutex<(BTreeSet<String>, Vec<String>)> = Mutex::new((BTreeSet::new(), Vec::new()));
    fn traced(key: &str) {
        if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
            let mut reads = READS.lock().unwrap_or_else(|e| e.into_inner());
            if reads.0.insert(key.to_owned()) {
                reads.1.push(key.to_owned());
            }
        }
    }
    pub fn trace_reads(on: bool) {
        TRACE.store(on, std::sync::atomic::Ordering::Relaxed);
    }
    /// Files opened for the first time since the last call.
    pub fn first_reads() -> Vec<String> {
        std::mem::take(&mut READS.lock().unwrap_or_else(|e| e.into_inner()).1)
    }

    pub fn key(path: &Path) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let text = path.to_string_lossy().replace('\\', "/");
        for part in text.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                other => parts.push(other),
            }
        }
        let owned: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
        owned.join("/")
    }
    fn missing(path: &Path) -> io::Error {
        io::Error::new(io::ErrorKind::NotFound, format!("{}: not found", path.display()))
    }
    fn tree() -> std::sync::MutexGuard<'static, Tree> {
        TREE.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn notify(key: &str, data: Option<&[u8]>) {
        if let Some(hook) = *HOOK.lock().unwrap_or_else(|e| e.into_inner()) {
            hook(key, data);
        }
    }

    pub fn mount(path: &str, bytes: Vec<u8>) {
        tree().files.insert(key(Path::new(path)), Arc::new(bytes));
    }
    /// Unmount a file and hand over its bytes without copying them (when
    /// nothing else holds them), for a reader that consumes the whole file.
    pub fn take(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        traced(&key(path));
        let data = tree().files.remove(&key(path)).ok_or_else(|| missing(path))?;
        Ok(Arc::try_unwrap(data).unwrap_or_else(|shared| shared.as_ref().clone()))
    }
    /// Record a file the host holds without mounting its bytes.
    pub fn declare(path: &str) {
        tree().declared.insert(key(Path::new(path)));
    }
    /// A declared file whose bytes are not mounted yet.
    pub fn needs_fetch(path: impl AsRef<Path>) -> bool {
        let key = key(path.as_ref());
        let tree = tree();
        tree.declared.contains(&key) && !tree.files.contains_key(&key)
    }
    pub fn unmount_prefix(prefix: &str) {
        let prefix = key(Path::new(prefix));
        let mut tree = tree();
        tree.files.retain(|k, _| k != &prefix && !k.starts_with(&format!("{prefix}/")));
    }
    pub fn mounted(path: &str) -> bool {
        tree().files.contains_key(&key(Path::new(path)))
    }
    pub fn mounted_bytes() -> usize {
        tree().files.values().map(|v| v.len()).sum()
    }
    pub fn set_write_hook(hook: fn(&str, Option<&[u8]>)) {
        *HOOK.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
    }

    pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
        let path = path.as_ref();
        traced(&key(path));
        tree().files.get(&key(path)).map(|v| v.as_ref().clone()).ok_or_else(|| missing(path))
    }
    pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
        String::from_utf8(read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }
    pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
        let key = key(path.as_ref());
        notify(&key, Some(contents.as_ref()));
        tree().files.insert(key, Arc::new(contents.as_ref().to_vec()));
        Ok(())
    }
    pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
        tree().dirs.insert(key(path.as_ref()));
        Ok(())
    }
    pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let from = from.as_ref();
        let (from_key, to_key) = (key(from), key(to.as_ref()));
        let data = tree().files.remove(&from_key).ok_or_else(|| missing(from))?;
        notify(&from_key, None);
        notify(&to_key, Some(data.as_ref()));
        tree().files.insert(to_key, data);
        Ok(())
    }
    pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let key = key(path);
        tree().files.remove(&key).ok_or_else(|| missing(path))?;
        notify(&key, None);
        Ok(())
    }
    pub fn is_file(path: impl AsRef<Path>) -> bool {
        let key = key(path.as_ref());
        let tree = tree();
        tree.files.contains_key(&key) || tree.declared.contains(&key)
    }
    pub fn is_dir(path: impl AsRef<Path>) -> bool {
        let key = key(path.as_ref());
        let tree = tree();
        if key.is_empty() || tree.dirs.contains(&key) {
            return true;
        }
        let prefix = format!("{key}/");
        tree.files.range(prefix.clone()..).next().is_some_and(|(k, _)| k.starts_with(&prefix))
            || tree.declared.range(prefix.clone()..).next().is_some_and(|k| k.starts_with(&prefix))
    }
    pub fn exists(path: impl AsRef<Path>) -> bool {
        is_file(path.as_ref()) || is_dir(path.as_ref())
    }
    pub fn canonicalize(path: impl AsRef<Path>) -> io::Result<PathBuf> {
        let path = path.as_ref();
        if !exists(path) {
            return Err(missing(path));
        }
        Ok(PathBuf::from(key(path)))
    }
    pub fn read_dir(path: impl AsRef<Path>) -> io::Result<Vec<DirEntry>> {
        let path = path.as_ref();
        if !is_dir(path) {
            return Err(missing(path));
        }
        let base = key(path);
        let prefix = if base.is_empty() { String::new() } else { format!("{base}/") };
        let tree = tree();
        let mut names: BTreeMap<String, bool> = BTreeMap::new();
        let files = tree.files.range(prefix.clone()..).map(|(k, _)| k).take_while(|k| k.starts_with(&prefix));
        let declared = tree.declared.range(prefix.clone()..).take_while(|k| k.starts_with(&prefix));
        for k in files.chain(declared) {
            let rest = &k[prefix.len()..];
            match rest.split_once('/') {
                Some((dir, _)) => {
                    names.insert(dir.to_owned(), true);
                }
                None => {
                    names.entry(rest.to_owned()).or_insert(false);
                }
            }
        }
        for d in tree.dirs.range(prefix.clone()..).take_while(|d| d.starts_with(&prefix)) {
            if let Some((dir, _)) = d[prefix.len()..].split_once('/').or(Some((&d[prefix.len()..], ""))) {
                if !dir.is_empty() {
                    names.insert(dir.to_owned(), true);
                }
            }
        }
        Ok(names
            .into_iter()
            .map(|(name, is_dir)| DirEntry { path: Path::new(&base).join(name), is_dir })
            .collect())
    }
}

pub use imp::*;

/// One directory child. `path` is the parent joined with the child name.
#[derive(Clone, Debug)]
pub struct DirEntry {
    pub path: PathBuf,
    pub is_dir: bool,
}

impl DirEntry {
    /// Mirrors `std::fs::DirEntry::path` so call sites stay identical.
    pub fn path(&self) -> PathBuf {
        self.path.clone()
    }

    pub fn file_name(&self) -> std::ffi::OsString {
        self.path.file_name().map(|n| n.to_os_string()).unwrap_or_default()
    }
}
