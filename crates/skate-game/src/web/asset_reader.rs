//! Bevy asset source backed by the in-memory VFS, falling back to the host's
//! browser storage (OPFS) so large files never need to be mounted up front.
use super::host;
use bevy::asset::io::{AssetReader, AssetReaderError, PathStream, Reader, VecReader};
use std::path::{Path, PathBuf};

fn get_meta_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".meta");
    PathBuf::from(name)
}

pub(crate) struct VfsAssetReader {
    root: PathBuf,
}

impl VfsAssetReader {
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self { root: root.as_ref().to_owned() }
    }
}

async fn fetch(path: PathBuf) -> Result<VecReader, AssetReaderError> {
    if let Ok(bytes) = skate_data::vfs::read(&path) {
        return Ok(VecReader::new(bytes));
    }
    let key = path.to_string_lossy().replace('\\', "/");
    match host::read(&key).await {
        Ok(Some(bytes)) => Ok(VecReader::new(bytes)),
        Ok(None) => Err(AssetReaderError::NotFound(path)),
        Err(error) => Err(AssetReaderError::Io(std::io::Error::other(error).into())),
    }
}

impl AssetReader for VfsAssetReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        fetch(self.root.join(path)).await
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        fetch(get_meta_path(&self.root.join(path))).await
    }

    async fn read_directory<'a>(&'a self, _path: &'a Path) -> Result<Box<PathStream>, AssetReaderError> {
        let stream: Box<PathStream> = Box::new(bevy::tasks::futures_lite::stream::empty());
        Ok(stream)
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}
