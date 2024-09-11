use crate::scope::Scope;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Copy variables from the current scope outwards into the environment of the
/// command to be executed, but do not carry over special redirect variables.
pub fn copy_vars_to_command_env(command: &mut std::process::Command, scope: &Arc<Scope>) {
    // Override existing environment variables
    command.env_clear();

    let mut current_scope = Some(scope);
    while let Some(scope) = &current_scope {
        for (key, variable) in scope.vars().iter() {
            if !key.is_special_var() {
                command.env(&key.view(), variable.value().to_string());
            }
        }
        current_scope = scope.parent.as_ref();
    }
}

/// Clear the environment, and copy variables from scope into environment.
pub fn sync_env_vars(scope: &Scope) {
    // Remove each environment variable
    env::vars().for_each(|(key, _)| env::remove_var(key));

    for (key, var) in scope.vars().iter() {
        env::set_var(key.as_str(), var.to_string());
    }
}

/// Get the interpreter's own path, working around test mode.
/// This function is used when evaluating pipe expressions;
/// the expression of the right hand-side of a pipe is passed to
/// a new interpreter instance, with the stdin reading from the
/// left hand-side of the pipe.
/// It is also used by the "sudo" implementation on Windows.
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
                    Ok(re.replace(&path_str, "\\shmy$1").to_string())
                }
                #[cfg(not(windows))]
                {
                    let re = Regex::new(r"/deps/.+?(\..*)?$").map_err(|e| e.to_string())?;
                    Ok(re.replace(&path_str, "/shmy$1").to_string())
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

/// Format file / disk usage sizes, using units (K, M, etc) when the human_readable
/// flag is true. Use old-school 1024 as orders of magnitude instead of 1000.
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
    use crate::symlnk::SymLink;
    use std::fs::{self, OpenOptions};
    use std::io;
    use std::mem;
    use std::os::windows::prelude::*;
    use std::path::{Path, PathBuf};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::{
        Win32::Storage::FileSystem::{
            FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
        Win32::System::Ioctl::{FSCTL_DELETE_REPARSE_POINT, FSCTL_GET_REPARSE_POINT},
        Win32::System::IO::DeviceIoControl,
    };

    pub const IO_REPARSE_TAG_LX_SYMLINK: u32 = 0xA000001D;
    pub const MAX_REPARSE_DATA_BUFFER_SIZE: usize = 16 * 1024;

    #[repr(C)]
    pub struct ReparseHeader {
        pub reparse_tag: u32,
        pub data_length: u16,
        reserved: u16,
    }

    // IO_REPARSE_TAG_LX_SYMLINK reparse data structure
    #[repr(C)]
    #[derive(Debug)]
    pub struct ReparseDataBufferLxSymlink {
        pub reparse_tag: u32,
        pub data_length: u16,
        reserved: u16,
        unused: u16,             // Not sure what this field is
        reparse_target: [u8; 1], // Variable-length
    }

    // Not Windows-specific, just used by the df and du windows impls.
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

    pub fn read_reparse_data<'a, D: Sized>(
        path: &Path,
        buffer: &'a mut Vec<u8>,
    ) -> io::Result<&'a mut D> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
            .access_mode(FILE_READ_ATTRIBUTES.0)
            .open(&path)?;

        let mut bytes_returned = 0;

        // Retrieve the reparse point data
        unsafe {
            DeviceIoControl(
                HANDLE(file.as_raw_handle()),
                FSCTL_GET_REPARSE_POINT,
                None,
                0,
                Some(buffer.as_mut_ptr() as *mut _),
                buffer.len() as u32,
                Some(&mut bytes_returned),
                None,
            )
        }
        .map_err(|_| io::Error::last_os_error())?;

        if bytes_returned < mem::size_of::<D>() as u32 {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid reparse point data",
            ))
        } else {
            // Cast the buffer into a reference of type D
            let header = unsafe { &mut *(buffer.as_mut_ptr() as *mut D) };

            Ok(header)
        }
    }

    /// Read WSL symbolic link.
    /// If a non-WSL link is detected, fail over to fs::read_link
    pub fn read_link(path: &Path) -> std::io::Result<PathBuf> {
        const WSL_LINK_SIZE: usize = mem::size_of::<ReparseDataBufferLxSymlink>();

        // Prepare buffer for reparse point data
        let mut buffer: Vec<u8> = vec![0; MAX_REPARSE_DATA_BUFFER_SIZE];

        match read_reparse_data::<ReparseDataBufferLxSymlink>(path, &mut buffer) {
            Ok(data) => {
                // Defer to the normal fs operation if not a Linux symlink
                if data.reparse_tag != IO_REPARSE_TAG_LX_SYMLINK {
                    return fs::read_link(path);
                }

                let target_length = std::cmp::min(
                    data.data_length.saturating_sub(4) as usize,
                    buffer.len() - WSL_LINK_SIZE,
                );
                let target = &buffer[WSL_LINK_SIZE..][..target_length];

                Ok(String::from_utf8_lossy(target)
                    .into_owned()
                    .replace("/", "\\")
                    .into())
            }
            Err(e) => Err(e),
        }
    }

    /// Read the parse point with FSCTL_GET_REPARSE_POINT,
    /// use FSCTL_DELETE_REPARSE_POINT to remove symbolic link,
    /// then remove the file or directory given by `path`.
    pub fn remove_link(path: &Path) -> std::io::Result<()> {
        let is_dir = path.resolve()?.is_dir();

        // lifetime scope for the file to close automatically
        {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | FILE_FLAG_OPEN_REPARSE_POINT.0)
                .open(&path)?;

            let handle = HANDLE(file.as_raw_handle());

            let mut buffer: Vec<u8> = vec![0; MAX_REPARSE_DATA_BUFFER_SIZE];

            // First read the parse point, because the tag passed to
            // FSCTL_DELETE_REPARSE_POINT must match the existing one.
            let header = read_reparse_data::<ReparseHeader>(path, &mut buffer)?;
            let mut bytes_returned = std::mem::size_of::<ReparseHeader>() as u32;

            // Clear the data_length
            // https://learn.microsoft.com/en-us/windows-hardware/drivers/ifs/fsctl-delete-reparse-point
            header.data_length = 0;

            let header_size = if header.reparse_tag == IO_REPARSE_TAG_LX_SYMLINK {
                8
            } else {
                24
            };
            unsafe {
                DeviceIoControl(
                    handle,
                    FSCTL_DELETE_REPARSE_POINT,
                    Some(buffer.as_mut_ptr() as *mut _),
                    std::cmp::min(header_size, bytes_returned),
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

    pub fn is_elevated() -> io::Result<bool> {
        unsafe {
            // Open the process token
            let process_handle = GetCurrentProcess();
            let mut token_handle = HANDLE::default();

            match OpenProcessToken(process_handle, TOKEN_QUERY, &mut token_handle) {
                Ok(_) => {
                    let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
                    let mut return_length = 0;

                    // Query the token elevation
                    let result = GetTokenInformation(
                        token_handle,
                        TokenElevation,
                        Some(&mut elevation as *mut _ as *mut std::ffi::c_void),
                        std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                        &mut return_length,
                    );

                    CloseHandle(token_handle).unwrap_or(());

                    match result {
                        Ok(_) => Ok(elevation.TokenIsElevated != 0),
                        Err(_) => Err(std::io::Error::last_os_error()),
                    }
                }
                Err(_) => Err(std::io::Error::last_os_error()),
            }
        }
    }
}

/// Return the target of a symbolic link.
pub fn read_symlink(path: &Path) -> io::Result<PathBuf> {
    #[cfg(not(windows))]
    {
        fs::read_link(path)
    }
    #[cfg(windows)]
    {
        win::read_link(path).or_else(|_| fs::read_link(path))
    }
}

/// Keep reading symbolic links until either non-link or cycle is detected.
pub fn resolve_links(path: &Path) -> io::Result<PathBuf> {
    use std::collections::HashSet;

    let mut visited = HashSet::new();
    let mut path = path.to_path_buf();

    while path.is_symlink() {
        if !visited.insert(path.clone()) {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("Cyclical symbolic link: {}", path.display()),
            ));
        }
        path = read_symlink(&path)?;
    }

    Ok(path)
}

pub fn format_error<E: std::fmt::Display>(
    scope: &Scope,
    value: &str,
    args: &[String],
    error: E,
) -> String {
    format!("{}: {}", scope.err_path_arg(value, args), error)
}
