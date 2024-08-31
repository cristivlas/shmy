#[cfg(windows)]
use crate::utils::win;
use std::io;
use std::path::Path;

pub trait IsWslLink {
    fn is_wsl_link(&self) -> io::Result<bool>;
}

#[cfg(windows)]
impl IsWslLink for Path {
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
}

#[cfg(not(windows))]
impl IsWslLink for Path {
    fn is_wsl_link(&self) -> io::Result<bool> {
        Ok(false)
    }
}
