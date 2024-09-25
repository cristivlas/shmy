use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink, utils::format_error};
use regex::Regex;
use std::borrow::Cow;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct Find {
    flags: CommandFlags,
}

impl Find {
    fn new() -> Self {
        let flags = CommandFlags::with_help();
        Self { flags }
    }

    fn search(
        &self,
        scope: &Arc<Scope>,
        file_name: &OsStr,
        path: &Path,
        regex: &Regex,
        visited: &mut HashSet<String>,
    ) -> Result<(), String> {
        if Scope::is_interrupted() {
            return Ok(());
        }

        let search_path = path.dereference().unwrap_or(Cow::Owned(path.into()));

        if !visited.insert(search_path.to_string_lossy().to_string()) {
            return Ok(()); // Already seen
        }

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
                                    visited,
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let search_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: find [OPTIONS] [DIRS...] PATTERN");
            println!("Recursively search and print paths matching PATTERN.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if search_args.is_empty() {
            return Err("Missing search pattern".to_string());
        }

        let pattern = search_args.last().unwrap(); // Last argument is the search pattern
        let regex = Regex::new(pattern).map_err(|e| format!("Invalid regex: {}", e))?;

        let dirs = if search_args.len() > 1 {
            &search_args[..search_args.len() - 1] // All except the last
        } else {
            &vec![String::from(".")] // Default to current directory
        };

        let mut visited = HashSet::new();

        for dir in dirs {
            let path = Path::new(dir)
                .dereference()
                .map_err(|e| format_error(&scope, dir, args, e))?;

            self.search(scope, OsStr::new(dir), &path, &regex, &mut visited)?;
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
