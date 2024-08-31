use std::io;
use std::path::{Path, PathBuf};

pub trait SymLink {
    fn is_wsl_link(&self) -> io::Result<bool>;
    fn resolve(&self) -> io::Result<PathBuf>;
}

#[cfg(not(windows))]
fn resolve_path(path: &Path) -> io::Result<PathBuf> {
    path.canonicalize()
}

/// Resolve symbolic links, including WSL links, which
/// are not handled by fs::canonicalize on Windows.
#[cfg(windows)]
fn resolve_path(sym_path: &Path) -> io::Result<PathBuf> {
    use crate::utils::resolve_links;
    use std::borrow::Cow;
    use std::collections::HashMap;
    use std::env;
    use std::path::Component;

    // map paths with possible symlink components to resolved
    let mut visited: HashMap<PathBuf, PathBuf> = HashMap::new();

    let mut path = if sym_path.is_absolute() {
        PathBuf::new()
    } else {
        env::current_dir()?
    };

    for component in sym_path.components() {
        match component {
            // Component::RootDir => path.push(component),
            Component::CurDir => continue,
            Component::ParentDir => {
                path.pop();
            }
            _ => path.push(component),
        }
        let resolved = {
            if let Some(p) = visited.get(&path) {
                Cow::<'_, PathBuf>::Borrowed(p)
            } else {
                let partial_resolved = resolve_links(&path)?;
                visited.insert(path.clone(), partial_resolved.clone());

                Cow::<'_, PathBuf>::Owned(partial_resolved)
            }
        };
        // dbg!(&path, &resolved);

        if resolved.is_absolute() {
            path = resolved.into_owned();
        } else {
            path.pop();
            path.push(&*resolved);
        }
    }

    // Do not canonicalize here, to avoid UNC trouble
    Ok(path)
}

impl SymLink for Path {
    #[cfg(not(windows))]
    fn is_wsl_link(&self) -> io::Result<bool> {
        Ok(false)
    }

    #[cfg(windows)]
    fn is_wsl_link(&self) -> io::Result<bool> {
        use crate::utils::win;

        if !self.is_symlink() {
            Ok(false)
        } else {
            const BUF_SIZE: usize = 1024;
            let mut buf: Vec<u8> = vec![0; BUF_SIZE];
            let hdr = win::read_reparse_data::<win::ReparseHeader>(self, &mut buf)?;

            Ok(hdr.reparse_tag == win::IO_REPARSE_TAG_LX_SYMLINK)
        }
    }

    fn resolve(&self) -> io::Result<PathBuf> {
        resolve_path(self)
    }
}

impl SymLink for PathBuf {
    fn is_wsl_link(&self) -> io::Result<bool> {
        self.as_path().is_wsl_link()
    }

    fn resolve(&self) -> io::Result<PathBuf> {
        resolve_path(self)
    }
}
