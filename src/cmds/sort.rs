use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::{eval::Value, scope::Scope, utils::format_error};
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;
use std::sync::Arc;

struct Sort {
    flags: CommandFlags,
}

impl Sort {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('u', "unique", "Output only the first of an equal run");
        flags.add_flag('r', "reverse", "Reverse the result of comparisons");
        flags.add_flag(
            'n',
            "numeric-sort",
            "Compare according to string numerical value",
        );
        Self { flags }
    }

    fn sort_lines(
        &self,
        lines: Vec<String>,
        unique: bool,
        reverse: bool,
        numeric: bool,
    ) -> Vec<String> {
        let mut sorted_lines: Vec<String> = if unique {
            lines
                .into_iter()
                .collect::<HashSet<_>>()
                .into_iter()
                .collect()
        } else {
            lines
        };

        if numeric {
            sorted_lines.sort_by(|a, b| {
                let a_num = a.parse::<f64>().unwrap_or(f64::MAX);
                let b_num = b.parse::<f64>().unwrap_or(f64::MAX);
                a_num.partial_cmp(&b_num).unwrap()
            });
        } else {
            sorted_lines.sort();
        }

        if reverse {
            sorted_lines.reverse();
        }

        sorted_lines
    }
}

impl Exec for Sort {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: sort [OPTIONS] [FILE]...");
            println!("Sort lines of text (from FILES or standard input).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let unique = flags.is_present("unique");
        let reverse = flags.is_present("reverse");
        let numeric = flags.is_present("numeric-sort");

        let mut lines = Vec::new();

        if args.is_empty() {
            // Read from stdin if no files are provided
            scope.show_eof_hint();
            let reader = io::stdin().lock();
            for line in reader.lines() {
                if Scope::is_interrupted() {
                    break;
                }
                let line = line.map_err(|e| e.to_string())?;
                lines.push(line);
            }
        } else {
            for file_path in &args {
                let path = Path::new(file_path)
                    .dereference()
                    .map_err(|e| format_error(scope, file_path, &args, e))?;

                if path.is_file() {
                    match fs::read_to_string(&path) {
                        Ok(content) => {
                            if Scope::is_interrupted() {
                                break;
                            }
                            lines.extend(content.lines().map(String::from));
                        }
                        Err(e) => {
                            my_warning!(scope, "Cannot read {}: {}", scope.err_path(&path), e)
                        }
                    }
                } else if path.is_dir() {
                    my_warning!(scope, "{}: Is a directory", scope.err_path(&path));
                } else {
                    my_warning!(scope, "{}: Is not a regular file", scope.err_path(&path));
                }
            }
        }

        let sorted_lines = self.sort_lines(lines, unique, reverse, numeric);

        for line in sorted_lines {
            if Scope::is_interrupted() {
                break;
            }

            my_println!("{line}")?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "sort".to_string(),
        inner: Arc::new(Sort::new()),
    });
}
