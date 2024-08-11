use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use regex::Regex;
use std::fs;
use std::io::{self, BufRead};
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
        Grep { flags }
    }

    fn process_line(
        line_number: usize,
        line: &str,
        regex: &Regex,
        line_number_flag: bool,
        ignore_case: bool,
    ) {
        let line_to_check = if ignore_case {
            line.to_lowercase()
        } else {
            line.to_string()
        };

        if regex.is_match(&line_to_check) {
            if line_number_flag {
                println!("{}: {}", line_number + 1, line);
            } else {
                println!("{}", line);
            }
        }
    }
}

impl Exec for Grep {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
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

        let regex = if ignore_case {
            Regex::new(&pattern.to_lowercase()).map_err(|e| e.to_string())?
        } else {
            Regex::new(pattern).map_err(|e| e.to_string())?
        };

        let files = &args[1..];
        if files.is_empty() {
            // Read from stdin if no files are provided
            let stdin = io::stdin();
            let reader = stdin.lock();
            for (line_number, line) in reader.lines().enumerate() {
                let line = line.map_err(|e| e.to_string())?;
                Self::process_line(line_number, &line, &regex, line_number_flag, ignore_case);
            }
        } else {
            for file in files {
                let path = Path::new(file);
                let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                for (line_number, line) in content.lines().enumerate() {
                    Self::process_line(line_number, &line, &regex, line_number_flag, ignore_case);
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
