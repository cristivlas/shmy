use crate::scope::Scope;
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Copy variables from the current scope outwards into the environment of the
/// command to be executed, but do not carry over special redirect variables.
pub fn copy_vars_to_command_env(command: &mut std::process::Command, scope: &Rc<Scope>) {
    // Override existing environment variables
    command.env_clear();

    let mut current_scope = Some(scope);
    while let Some(scope) = &current_scope {
        for (key, variable) in scope.vars.borrow().iter() {
            if !key.is_special_var() {
                command.env(&key.view(), variable.value().to_string());
            }
        }
        current_scope = scope.parent.as_ref();
    }
}

pub fn sync_env_vars(scope: &Scope) {
    // Remove each environment variable
    env::vars().for_each(|(key, _)| env::remove_var(key));

    for (key, var) in scope.vars.borrow().iter() {
        env::set_var(key.as_str(), var.to_string());
    }
}

/// Get our own path
pub fn executable() -> Result<String, String> {
    match env::current_exe() {
        Ok(p) => {
            #[cfg(test)]
            {
                use regex::Regex;

                let path_str = p.to_string_lossy();
                #[cfg(windows)]
                {
                    let re = Regex::new(r"\\deps\\.*?(\..*)?$").map_err(|e| e.to_string())?;
                    Ok(re.replace(&path_str, "\\mysh$1").to_string())
                }
                #[cfg(not(windows))]
                {
                    let re = Regex::new(r"/deps/.+?(\..*)?$").map_err(|e| e.to_string())?;
                    Ok(re.replace(&path_str, "/mysh$1").to_string())
                }
            }
            #[cfg(not(test))]
            {
                Ok(p.to_string_lossy().to_string())
            }
        }
        Err(e) => Err(format!("Failed to get executable name: {}", e)),
    }
}

pub fn format_size(size: u64, block_size: u64, human_readable: bool) -> String {
    if !human_readable {
        return (size / block_size).to_string();
    }

    let units = ["B", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let mut index = 0;
    let mut formatted_size = size as f64;

    while formatted_size >= 1024.0 && index < units.len() - 1 {
        formatted_size /= 1024.0;
        index += 1;
    }

    format!("{:.1} {}", formatted_size, units[index])
}

#[cfg(windows)]
pub mod win {
    use std::fs::{self, OpenOptions};
    use std::os::windows::prelude::*;
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::HANDLE;
    use windows::{
        Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
        Win32::System::Ioctl::{FSCTL_DELETE_REPARSE_POINT, FSCTL_GET_REPARSE_POINT},
        Win32::System::IO::DeviceIoControl,
    };
    const MAX_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;

    pub fn root_path(path: &Path) -> PathBuf {
        let mut path = path.to_path_buf();

        if let Some(root) = path.components().next() {
            path = root.as_os_str().to_os_string().into();
            path.push("/");
            path
        } else {
            PathBuf::from("/")
        }
    }

    /// Read WSL symbolic link.
    /// If a non-WSL link is detected, fail over to fs::read_link
    pub fn read_link(path: &Path) -> std::io::Result<PathBuf> {
        const IO_REPARSE_TAG_LX_SYMLINK: u32 = 0xA000001D;

        // IO_REPARSE_TAG_LX_SYMLINK reparse data structure
        #[repr(C)]
        #[derive(Debug)]
        struct ReparseDataBufferLxSymlink {
            reparse_tag: u32,
            data_length: u16,
            reserved: u16,
            unused: u16,             // Not sure what this field is
            reparse_target: [u8; 1], // Variable-length
        }

        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .open(&path)?;

        let handle = HANDLE(file.as_raw_handle());

        // Prepare buffer for reparse point data
        let mut buffer: Vec<u8> = vec![0; MAX_REPARSE_DATA_BUFFER_SIZE];

        let mut bytes_returned = 0;

        // Retrieve the reparse point data
        unsafe {
            DeviceIoControl(
                handle,
                FSCTL_GET_REPARSE_POINT,
                None,
                0,
                Some(buffer.as_mut_ptr() as *mut _),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .map_err(|_| std::io::Error::last_os_error())?;

        let reparse_data = unsafe { &*(buffer.as_ptr() as *const ReparseDataBufferLxSymlink) };

        // Defer to the normal fs operation if not a Linux symlink
        if reparse_data.reparse_tag != IO_REPARSE_TAG_LX_SYMLINK {
            return fs::read_link(path);
        }

        let target_length = std::cmp::min(
            reparse_data.data_length.saturating_sub(4) as usize,
            buffer.len() - std::mem::size_of_val(reparse_data),
        );
        let target = &buffer[std::mem::size_of_val(reparse_data)..][..target_length];

        Ok(String::from_utf8_lossy(target).into_owned().into())
    }

    /// Read the parse point with FSCTL_GET_REPARSE_POINT,
    /// use FSCTL_DELETE_REPARSE_POINT to remove symbolic link,
    /// then remove the file or directory given by `path`.
    pub fn remove_link(path: &Path) -> std::io::Result<()> {
        let is_dir = path.is_dir();
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(&path)?;

            let handle = HANDLE(file.as_raw_handle());

            let mut buffer: Vec<u8> = vec![0; MAX_REPARSE_DATA_BUFFER_SIZE];
            let mut bytes_returned = 0;

            // First, read the parse point, because the tag passed to
            // FSCTL_DELETE_REPARSE_POINT must match.
            unsafe {
                DeviceIoControl(
                    handle,
                    FSCTL_GET_REPARSE_POINT,
                    None,
                    0,
                    Some(buffer.as_mut_ptr() as *mut _),
                    buffer.len() as u32,
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .map_err(|_| std::io::Error::last_os_error())?;

            #[repr(C)]
            struct ReparseHeader {
                reparse_tag: u32,
                data_length: u16,
                reserved: u16,
            }
            // Clear the data_length
            // https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/fsctl-delete-reparse-point
            let header: &mut ReparseHeader =
                unsafe { &mut *(buffer.as_mut_ptr() as *mut ReparseHeader) };
            header.data_length = 0;

            unsafe {
                DeviceIoControl(
                    handle,
                    FSCTL_DELETE_REPARSE_POINT,
                    Some(buffer.as_mut_ptr() as *mut _),
                    std::cmp::min(24, bytes_returned),
                    None,
                    0 as _,
                    Some(&mut bytes_returned),
                    None,
                )
            }
            .map_err(|_| std::io::Error::last_os_error())?;
        }

        // Finally, remove the file or directory
        if is_dir {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        }
    }
}

pub fn read_symlink(path: &Path) -> Result<PathBuf, String> {
    #[cfg(not(windows))]
    {
        fs::read_link(path).map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        win::read_link(path)
            .or_else(|_| fs::read_link(path))
            .map_err(|e| e.to_string())
    }
}

pub fn resolve_links(path: &Path) -> Result<PathBuf, String> {
    let mut visited_paths = HashSet::new();
    let mut path = path.to_path_buf();

    while path.is_symlink() {
        if !visited_paths.insert(path.clone()) {
            return Err(format!(
                "Cycle detected in symbolic links: {}",
                path.display()
            ));
        }
        path = read_symlink(&path)?;
    }

    Ok(path)
}
