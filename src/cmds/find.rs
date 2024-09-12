use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::{eval::Value, scope::Scope};
use regex::Regex;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct Find {
    flags: CommandFlags,
}

impl Find {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('L', "follow-links", "Follow symbolic links");

        Self { flags }
    }

    fn search(
        &self,
        scope: &Arc<Scope>,
        file_name: &OsStr,
        path: &Path,
        regex: &Regex,
        follow: bool,
    ) -> Result<(), String> {
        if Scope::is_interrupted() {
            return Ok(());
        }

        let search_path = if follow {
            path.resolve().unwrap_or(path.to_path_buf())
        } else {
            path.to_path_buf()
        };

        // Check if the current directory or file matches the pattern
        if regex.is_match(&file_name.to_string_lossy()) {
            println!("{}", path.display());
        }

        if search_path.is_dir() {
            match fs::read_dir(search_path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => {
                                self.search(
                                    scope,
                                    &entry.file_name(),
                                    &entry.path(),
                                    regex,
                                    follow,
                                )?;
                            }
                            Err(e) => {
                                my_warning!(scope, "{}: {}", scope.err_path(path), e);
                            }
                        }
                    }
                }
                Err(e) => {
                    my_warning!(scope, "{}: {}", scope.err_path(path), e);
                }
            }
        }

        Ok(())
    }
}

impl Exec for Find {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: find [OPTIONS] [DIRS...] PATTERN");
            println!("Recursively search and print paths matching PATTERN.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing search pattern".to_string());
        }

        let follow_links = flags.is_present("follow-links");
        let pattern = args.last().unwrap(); // Last argument is the search pattern
        let regex = Regex::new(pattern).map_err(|e| format!("Invalid regex: {}", e))?;
        let dirs = if args.len() > 1 {
            &args[..args.len() - 1] // All except the last
        } else {
            &vec![String::from(".")] // Default to current directory
        };

        for dir in dirs {
            let path = Path::new(dir);
            self.search(scope, OsStr::new(dir), &path, &regex, follow_links)?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "find".to_string(),
        inner: Arc::new(Find::new()),
    });
}
