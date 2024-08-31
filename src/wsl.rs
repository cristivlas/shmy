#[cfg(windows)]
use crate::utils::resolve_links;
#[cfg(windows)]
use crate::utils::win;
use std::io;
use std::path::{Path, PathBuf};

pub trait WSL {
    fn is_wsl_link(&self) -> io::Result<bool>;
    fn resolve_links(&self) -> io::Result<PathBuf>;
}

#[cfg(windows)]
fn resolve(path_with_links: &Path) -> io::Result<PathBuf> {
    let mut path = PathBuf::new();

    for component in path_with_links.components() {
        path.push(component);
        let resolved = resolve_links(&path)?;
        if let Some(tail) = resolved.file_name() {
            path.pop();
            path.push(tail);
        }
    }
    Ok(path)
}

#[cfg(windows)]
impl WSL for Path {
    fn is_wsl_link(&self) -> io::Result<bool> {
        if !self.is_symlink() {
            Ok(false)
        } else {
            const BUF_SIZE: usize = 1024;
            let mut buf: Vec<u8> = vec![0; BUF_SIZE];
            let hdr = win::read_reparse_data::<win::ReparseHeader>(self, &mut buf)?;

            Ok(hdr.reparse_tag == win::IO_REPARSE_TAG_LX_SYMLINK)
        }
    }

    fn resolve_links(&self) -> io::Result<PathBuf> {
        resolve(self)
    }
}

#[cfg(windows)]
impl WSL for PathBuf {
    fn is_wsl_link(&self) -> io::Result<bool> {
        self.as_path().is_wsl_link()
    }

    fn resolve_links(&self) -> io::Result<PathBuf> {
        resolve(self)
    }
}

#[cfg(not(windows))]
impl WSL for Path {
    fn is_wsl_link(&self) -> io::Result<bool> {
        Ok(false)
    }

    fn resolve_links(&self) -> io::Result<PathBuf> {
        Ok(self.to_path_buf())
    }
}

#[cfg(not(windows))]
impl WSL for PathBuf {
    fn is_wsl_link(&self) -> io::Result<bool> {
        Ok(false)
    }

    fn resolve_links(&self) -> io::Result<PathBuf> {
        Ok(self.to_path_buf())
    }
}
