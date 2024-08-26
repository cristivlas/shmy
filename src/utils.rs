use crate::scope::Scope;
use std::env;
#[cfg(windows)]
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

pub fn sync_env_vars(scope: &Rc<Scope>) {
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
#[cfg(feature = "deprecated")]
pub fn win_last_err() -> String {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::GetLastError;
    use windows::Win32::System::Diagnostics::Debug::*;

    unsafe {
        let error_code = GetLastError();
        let mut buffer: Vec<u16> = Vec::with_capacity(512);

        let length = FormatMessageW(
            FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            None,
            error_code.0,
            0,
            PWSTR::null(),
            buffer.capacity() as u32,
            None,
        );

        if length == 0 {
            return format!("Unknown error: {}", error_code.0);
        }

        // Resize the buffer to the correct length
        buffer.set_len(length as usize);

        // Convert the UTF-16 buffer to an OsString
        let message = OsString::from_wide(&buffer);

        // Convert OsString to String, falling back to a lossy conversion if needed
        message.to_string_lossy().into_owned()
    }
}

#[cfg(windows)]
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
