use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use colored::*;
use regex::Regex;
use std::fs;
use std::io::{self, BufRead, IsTerminal};
use std::path::Path;
use std::rc::Rc;

struct Grep {
    flags: CommandFlags,
}

impl Grep {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag(
            'i',
            "ignore-case",
            "Ignore case distinctions in patterns and input data",
        );
        flags.add_flag(
            'n',
            "line-number",
            "Prefix each line of output with the 1-based line number",
        );
        flags.add_flag(
            'H',
            "with-filename",
            "Print the file name for each match (default when there is more than one file)",
        );
        flags.add_flag(
            'h',
            "no-filename",
            "Suppress the prefixing of file names on output",
        );
        Grep { flags }
    }

    fn process_line(
        filename: Option<&str>,
        line_number: usize,
        line: &str,
        regex: &Regex,
        line_number_flag: bool,
        ignore_case: bool,
        show_filename: bool,
        use_color: bool,
    ) {
        let line_to_check = if ignore_case {
            line.to_lowercase()
        } else {
            line.to_string()
        };

        if regex.is_match(&line_to_check) {
            let mut output = String::new();
            if show_filename {
                if let Some(name) = filename {
                    output.push_str(&format!("{}:", name));
                }
            }
            if line_number_flag {
                output.push_str(&format!("{}:", line_number + 1));
            }

            if use_color {
                let colored_line = regex.replace_all(line, |caps: &regex::Captures| {
                    caps[0].red().bold().to_string()
                });
                output.push_str(&colored_line);
            } else {
                output.push_str(line);
            }

            println!("{}", output);
        }
    }
}

impl Exec for Grep {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: grep [OPTIONS] PATTERN [FILE]...");
            println!("Search for PATTERN in each FILE (or stdin if no FILE is given).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing search pattern".to_string());
        }

        let pattern = &args[0];
        let ignore_case = flags.is_present("ignore-case");
        let line_number_flag = flags.is_present("line-number");
        let no_filename = flags.is_present("no-filename");
        let with_filename = flags.is_present("with-filename");
        let use_color = scope.lookup("NO_COLOR").is_none() && std::io::stdout().is_terminal();

        let regex = if ignore_case {
            Regex::new(&format!("(?i){}", pattern)).map_err(|e| e.to_string())?
        } else {
            Regex::new(pattern).map_err(|e| e.to_string())?
        };

        let files = &args[1..];
        let show_filename = if no_filename {
            false
        } else if with_filename || files.len() > 1 {
            true
        } else {
            false
        };

        if files.is_empty() {
            // Read from stdin if no files are provided
            let stdin = io::stdin();
            let reader = stdin.lock();
            for (line_number, line) in reader.lines().enumerate() {
                let line = line.map_err(|e| e.to_string())?;
                Self::process_line(
                    None,
                    line_number,
                    &line,
                    &regex,
                    line_number_flag,
                    ignore_case,
                    false,
                    use_color,
                );
            }
        } else {
            for file in files {
                let path = Path::new(file);
                let content = fs::read_to_string(&path)
                    .map_err(|e| format!("Cannot read '{}': {}", path.display(), e))?;
                for (line_number, line) in content.lines().enumerate() {
                    Self::process_line(
                        Some(file),
                        line_number,
                        line,
                        &regex,
                        line_number_flag,
                        ignore_case,
                        show_filename,
                        use_color,
                    );
                }
            }
        }
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "grep".to_string(),
        inner: Rc::new(Grep::new()),
    });
}
