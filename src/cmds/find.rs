use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use regex::Regex;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::rc::Rc;

struct Find {
    flags: CommandFlags,
}

impl Find {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        Find { flags }
    }

    fn search(
        &self,
        scope: &Rc<Scope>,
        file_name: &OsStr,
        path: &Path,
        regex: &Regex,
    ) -> Result<(), String> {
        if scope.is_interrupted() {
            return Ok(());
        }

        // Check if the current directory or file matches the pattern
        if regex.is_match(&file_name.to_string_lossy()) {
            println!("{}", path.display());
        }

        if path.is_dir() {
            match fs::read_dir(path) {
                Ok(entries) => {
                    for entry in entries {
                        match entry {
                            Ok(entry) => {
                                self.search(scope, &entry.file_name(), &entry.path(), regex)?;
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
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

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

        let pattern = args.last().unwrap(); // Last argument is the search pattern
        let regex = Regex::new(pattern).map_err(|e| format!("Invalid regex: {}", e))?;
        let dirs = if args.len() > 1 {
            &args[..args.len() - 1] // All except the last
        } else {
            &vec![String::from(".")] // Default to current directory
        };

        for dir in dirs {
            let path = Path::new(dir);
            self.search(scope, OsStr::new(dir), &path, &regex)?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "find".to_string(),
        inner: Rc::new(Find::new()),
    });
}
