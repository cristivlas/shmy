use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink, utils::format_error};
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct Chmod {
    flags: CommandFlags,
}

impl Chmod {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('r', "recursive", "Change permissions recursively");
        flags.add_flag('v', "verbose", "Report diagnostic for every file processed");

        Self { flags }
    }

    fn change_mode(
        path: &Path,
        mode: u32,
        recursive: bool,
        verbose: bool,
        scope: &Arc<Scope>,
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
                if Scope::is_interrupted() {
                    break;
                }
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

#[cfg(unix)]
fn help_details() {
    println!("\nExamples:");
    println!("  chmod 755 file     # Owner: rwx, Group: r-x, Others: r-x");
    println!("  chmod u+rwx file   # Add read, write, and execute for owner");
    println!("  chmod g+r file     # Add read permission for group");
    println!("  chmod o+r file     # Add read permission for others");
    println!("  chmod 644 file     # Owner: rw-, Group: r--, Others: r--");
    println!("  chmod 400 file     # Owner: r--, Group: ---, Others: --- (read-only)");
    println!("\nNote: Comma-separated mode lists are not supported.");
}

#[cfg(windows)]
fn help_details() {
    println!("\nExamples:");
    println!("  chmod u+rw file        # User gets read and write permissions");
    println!("  chmod -w file          # Make file read-only");

    println!("\nLimitations:");
    println!("  Windows does not support Unix-style group and others permissions.");
    println!("  Permissions must be explicitly set for specific users or groups via ACLs.");
    println!("       chmod g+r file     # No direct equivalent, need ACLs to modify group");
    println!("       chmod o+r file     # No direct equivalent, need ACLs to modify others");
    println!("  Future versions of this program may address these limitations.");
}

impl Exec for Chmod {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let paths = flags.parse_relaxed(scope, args);

        if flags.is_present("help") {
            println!("{}", "Usage: chmod [OPTIONS] MODE FILE...");
            println!("Change the mode (permissions) of each FILE to MODE.");
            println!("\nOptions:");
            println!("{}", flags.help());
            help_details();
            println!();

            return Ok(Value::success());
        }

        if paths.len() < 2 {
            return Err("Missing mode and file arguments".to_string());
        }

        let mode = Self::parse_mode(&paths[0])?;
        let recursive = flags.is_present("recursive");
        let verbose = flags.is_present("verbose");

        for arg in &paths[1..] {
            let path = Path::new(&arg)
                .dereference()
                .map_err(|e| format_error(scope, arg, &args, e))?;

            match Self::change_mode(&path, mode, recursive, verbose, scope) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("{}: {}", scope.err_path_arg(arg, args), e));
                }
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "chmod".to_string(),
        inner: Arc::new(Chmod::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;

    use tempfile::tempdir;

    #[cfg(unix)]
    #[test]
    fn test_chmod_unix_recursive() {
        let scope = Scope::new();
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile");
        fs::write(&file_path, "test content").unwrap();

        let result = Chmod::change_mode(&file_path, 0o644, false, false, &scope);
        assert!(result.is_ok());

        let permissions = fs::metadata(&file_path).unwrap().permissions();
        assert_eq!(permissions.mode() & 0o777, 0o644);
    }

    #[cfg(unix)]
    #[test]
    fn test_chmod_unix_recursive_directory() {
        let scope = Scope::new();
        let dir = tempdir().unwrap();
        let sub_dir = dir.path().join("subdir");
        let file_path = sub_dir.join("testfile");

        fs::create_dir(&sub_dir).unwrap();
        fs::write(&file_path, "test content").unwrap();

        let result = Chmod::change_mode(&sub_dir, 0o755, true, false, &scope);
        assert!(result.is_ok());

        let permissions = fs::metadata(&sub_dir).unwrap().permissions();
        assert_eq!(permissions.mode() & 0o777, 0o755);

        let file_permissions = fs::metadata(&file_path).unwrap().permissions();
        assert_eq!(file_permissions.mode() & 0o777, 0o755);
    }

    #[cfg(windows)]
    #[test]
    fn test_chmod_windows_readonly() {
        let scope = Scope::new();
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile");
        fs::write(&file_path, "test content").unwrap();

        let result = Chmod::change_mode(&file_path, 0o444, false, false, &scope);
        assert!(result.is_ok());

        let metadata = fs::metadata(&file_path).unwrap();
        let attributes = metadata.file_attributes();

        // Readonly attribute should be set
        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;
        assert!(attributes & FILE_ATTRIBUTE_READONLY.0 != 0);
    }

    #[cfg(windows)]
    #[test]
    fn test_chmod_windows_recursive() {
        let scope = Scope::new();
        let dir = tempdir().unwrap();
        let sub_dir = dir.path().join("subdir");
        let file_path = sub_dir.join("testfile");

        fs::create_dir(&sub_dir).unwrap();
        fs::write(&file_path, "test content").unwrap();

        let result = Chmod::change_mode(&sub_dir, 0o444, true, false, &scope);
        assert!(result.is_ok());

        let metadata = fs::metadata(&sub_dir).unwrap();
        let attributes = metadata.file_attributes();

        use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_READONLY;
        assert!(attributes & FILE_ATTRIBUTE_READONLY.0 != 0);

        let file_metadata = fs::metadata(&file_path).unwrap();
        let file_attributes = file_metadata.file_attributes();
        assert!(file_attributes & FILE_ATTRIBUTE_READONLY.0 != 0);
    }

    #[test]
    fn test_parse_mode() {
        let mode = Chmod::parse_mode("755").unwrap();
        assert_eq!(mode, 0o755);

        let mode = Chmod::parse_mode("u+rwx").unwrap();
        assert_eq!(mode, 0o700);

        let mode = Chmod::parse_mode("g+r").unwrap();
        assert_eq!(mode, 0o040);

        let mode = Chmod::parse_mode("o+r").unwrap();
        assert_eq!(mode, 0o004);
    }

    #[test]
    fn test_invalid_mode() {
        let result = Chmod::parse_mode("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_mode_or_file() {
        let scope = Scope::new();
        let chmod = Chmod::new();
        let result = chmod.exec("chmod", &vec![], &scope);
        assert!(result.is_err());
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_readonly_mode() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        fs::File::create(&file_path).unwrap();

        let scope = Scope::new();

        // Test setting file as read-only (mode: 0o444)
        Chmod::change_mode(&file_path, 0o444, false, false, &scope).unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        assert!(metadata.permissions().readonly());

        // Test setting write permissions (mode: 0o222)
        Chmod::change_mode(&file_path, 0o222, false, false, &scope).unwrap();
        let metadata = fs::metadata(&file_path).unwrap();
        assert!(!metadata.permissions().readonly()); // Should not be read-only anymore
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_symbolic_readonly_mode() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        fs::File::create(&file_path).unwrap();

        let scope = Scope::new();

        // Test setting file as read-only using symbolic mode (chmod u-w)
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("u+w").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap(); // Set to rw
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("u-w").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap(); // Set to r

        let result = fs::write(&file_path, "test");
        assert!(result.is_err()); // Expect it to fail since the file should be read-only.

        // Now remove write permissions for group and others
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("g-w").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap(); // Remove write for group
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("o-w").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap(); // Remove write for others

        let result = fs::write(&file_path, "test");
        assert!(result.is_err()); // Expect it to fail since the file should be read-only.

        // Confirm the file is still read-only
        let metadata = fs::metadata(&file_path).unwrap();
        assert!(metadata.permissions().readonly()); // Check if the file is read-only for all
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_symbolic_mode() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("testfile.txt");
        fs::File::create(&file_path).unwrap();

        let scope = Scope::new();

        // Use symbolic notation to set read-only for user
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("u+r").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap(); // User gets read
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("u+w").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap(); // User gets write

        let result = fs::write(&file_path, "test");
        assert!(result.is_ok()); // Expect it to succeed since user has write permission.

        // Now remove write permissions
        Chmod::change_mode(
            &file_path,
            Chmod::parse_mode("-w").unwrap(),
            false,
            false,
            &scope,
        )
        .unwrap();

        let result = fs::write(&file_path, "test");
        assert!(result.is_err()); // Expect it to fail

        // Confirm the file is still read-only
        let metadata = fs::metadata(&file_path).unwrap();
        assert!(metadata.permissions().readonly()); // Check if the file is read-only
    }
}
