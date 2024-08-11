use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use regex::Regex;
use std::fs;
use std::io::BufRead;
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
        Grep { flags }
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
        let regex = if flags.is_present("ignore-case") {
            Regex::new(&pattern.to_lowercase()).map_err(|e| e.to_string())?
        } else {
            Regex::new(pattern).map_err(|e| e.to_string())?
        };

        let files = &args[1..];
        if files.is_empty() {
            // Read from stdin if no files are provided
            let stdin = std::io::stdin();
            let reader = stdin.lock();
            let mut found_lines = Vec::new();
            for line in reader.lines() {
                let line = line.map_err(|e| e.to_string())?;
                if regex.is_match(&line) {
                    found_lines.push(line);
                }
            }
        } else {
            for file in files {
                let path = Path::new(file);
                let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                for line in content.lines() {
                    let line_to_check = if flags.is_present("ignore-case") {
                        line.to_lowercase()
                    } else {
                        line.to_string()
                    };

                    if regex.is_match(&line_to_check) {
                        println!("{}", line);
                    }
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
