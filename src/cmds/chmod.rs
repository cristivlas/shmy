use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::fs;
use std::path::Path;
use std::rc::Rc;

struct Chmod {
    flags: CommandFlags,
}

impl Chmod {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('r', "recursive", "Change permissions recursively");
        flags.add_flag('v', "verbose", "Report diagnostic for every file processed");

        Self { flags }
    }

    fn change_mode(
        path: &Path,
        mode: u32,
        recursive: bool,
        verbose: bool,
        scope: &Rc<Scope>,
    ) -> Result<(), String> {
        if verbose {
            println!("changing permissions of '{}' to {:o}", path.display(), mode);
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| {
                format!(
                    "Failed to change permissions of {}: {}",
                    scope.err_path(path),
                    error
                )
            })?;
        }

        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use std::os::windows::fs::MetadataExt;
            use windows::core::PWSTR;
            use windows::Win32::Storage::FileSystem::{
                SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_NORMAL,
                FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM, FILE_FLAGS_AND_ATTRIBUTES,
            };

            let metadata = fs::metadata(path).map_err(|error| {
                format!(
                    "Failed to get metadata for {}: {}",
                    scope.err_path(path),
                    error
                )
            })?;
            let mut attributes = metadata.file_attributes();

            // Clear the relevant attributes first
            attributes &=
                !(FILE_ATTRIBUTE_READONLY.0 | FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0);

            // Set read-only if write permission is not granted to anyone
            if mode & 0o222 == 0 {
                attributes |= FILE_ATTRIBUTE_READONLY.0;
            }

            // Windows doesn't have a direct equivalent for execute permissions
            // We're not setting hidden or system attributes based on the mode,
            // as these don't have a clear Unix equivalent

            // Ensure the file is not set to NORMAL, as it can't be combined with other attributes
            if attributes == FILE_ATTRIBUTE_NORMAL.0 {
                attributes = 0;
            }
            // Convert path to wide string
            let wide_path: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let wide_path_ptr = PWSTR::from_raw(wide_path.as_ptr() as *mut u16);

            // Use Windows API to set file attributes
            unsafe {
                SetFileAttributesW(wide_path_ptr, FILE_FLAGS_AND_ATTRIBUTES(attributes))
                    .map_err(|e| e.to_string())?;
            };
        }

        if recursive && path.is_dir() {
            for entry in fs::read_dir(path).map_err(|error| {
                format!(
                    "Failed to read directory {}: {}",
                    scope.err_path(path),
                    error
                )
            })? {
                let entry = entry.map_err(|error| {
                    format!(
                        "Failed to read directory entry in {}: {}",
                        scope.err_path(path),
                        error
                    )
                })?;

                let entry_path = entry.path();

                if entry_path.is_symlink() {
                    continue;
                }

                Self::change_mode(&entry_path, mode, recursive, verbose, scope)?;
            }
        }

        Ok(())
    }

    fn parse_mode(mode_str: &str) -> Result<u32, String> {
        if mode_str.chars().all(|c| c.is_digit(8)) {
            // Handle octal mode
            return u32::from_str_radix(mode_str, 8)
                .map_err(|_| format!("Invalid octal mode: {}", mode_str));
        }

        let mut result = 0;
        let mut who = 0;
        let mut action = ' ';
        let mut perm = 0;

        for c in mode_str.chars() {
            match c {
                'u' | 'g' | 'o' | 'a' => {
                    who |= match c {
                        'u' => 0o700,
                        'g' => 0o070,
                        'o' => 0o007,
                        'a' => 0o777,
                        _ => unreachable!(),
                    }
                }
                '+' | '-' | '=' => {
                    if action != ' ' {
                        Self::apply_change(&mut result, who, action, perm)?;
                        perm = 0;
                    }
                    action = c;
                }
                'r' => perm |= 0o444,
                'w' => perm |= 0o222,
                'x' => perm |= 0o111,
                'X' => perm |= 0o111, // For simplification, treat 'X' the same as 'x'
                's' => perm |= 0o4000 | 0o2000,
                't' => perm |= 0o1000,
                _ => return Err(format!("Invalid mode character: {}", c)),
            }
        }

        if action != ' ' {
            Self::apply_change(&mut result, who, action, perm)?;
        }

        Ok(result)
    }

    fn apply_change(mode: &mut u32, who: u32, action: char, perm: u32) -> Result<(), String> {
        if who == 0 {
            // Default to 'a' if no 'who' is specified
            *mode = match action {
                '+' => *mode | (perm & 0o7777),
                '-' => *mode & !(perm & 0o7777),
                '=' => perm & 0o7777,
                _ => return Err(format!("Invalid action: {}", action)),
            };
        } else {
            *mode = match action {
                '+' => *mode | (who & perm),
                '-' => *mode & !(who & perm),
                '=' => (*mode & !who) | (who & perm),
                _ => return Err(format!("Invalid action: {}", action)),
            };
        }
        Ok(())
    }
}

impl Exec for Chmod {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse_all(args);

        if flags.is_present("help") {
            println!("{}", "Usage: chmod [OPTIONS] MODE FILE...");
            println!("Change the mode (permissions) of each FILE to MODE.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.len() < 2 {
            return Err("Missing mode and file arguments".to_string());
        }

        let mode = Self::parse_mode(&args[0])?;
        let recursive = flags.is_present("recursive");
        let verbose = flags.is_present("verbose");

        for arg in &args[1..] {
            Self::change_mode(Path::new(&arg), mode, recursive, verbose, scope)?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "chmod".to_string(),
        inner: Rc::new(Chmod::new()),
    });
}
