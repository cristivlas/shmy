use super::{register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::utils::format_error;
use crate::{cmds::flags::CommandFlags, eval::Value, scope::Scope};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Mode {
    Cat,
    Head,
    Tail,
}

struct CatHeadTail {
    flags: CommandFlags,
    mode: Mode,
}

impl CatHeadTail {
    fn new(mode: Mode) -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('n', "number", "Number output lines");

        if matches!(mode, Mode::Head | Mode::Tail) {
            flags.add_option('l', "lines", "Specify the number of lines to output");
        }
        flags.add_flag('?', "help", "Display this help message");
        CatHeadTail { flags, mode }
    }

    fn mode_specific_help(&self) -> &str {
        match self.mode {
            Mode::Cat => "Concatenate FILE(s) to standard output.",
            Mode::Head => "Output the first part of files.",
            Mode::Tail => "Output the last part of files.",
        }
    }
}

impl Exec for CatHeadTail {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... [FILE]...", name);
            println!("{}", self.mode_specific_help());
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let line_num: bool = flags.is_present("number");
        let lines = flags
            .option("lines")
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|e| format_error(&scope, v, args, e))
            })
            .unwrap_or(Ok(10))?;

        let result = if filenames.is_empty() {
            scope.show_eof_hint();

            let mode = self.mode.clone();
            let mut stdin = BufReader::new(io::stdin());
            process_input(&mut stdin, mode, line_num, lines)
        } else {
            let mut result = Ok(());
            for filename in &filenames {
                let path = Path::new(filename)
                    .resolve()
                    .map_err(|e| format_error(&scope, filename, args, e))?;

                let mode = self.mode.clone();
                let file =
                    File::open(&path).map_err(|e| format_error(&scope, filename, args, e))?;

                let mut reader = BufReader::new(file);
                result = process_input(&mut reader, mode, line_num, lines);

                if result.is_err() {
                    break;
                }
            }
            result
        };

        result?;
        Ok(Value::success())
    }
}

fn process_input<R: BufRead>(
    reader: &mut R,
    mode: Mode, // Cat, Head or Tail
    line_numbers: bool,
    lines: usize,
) -> Result<(), String> {
    let mut i = 0;
    let mut tail = VecDeque::with_capacity(lines);

    for line in reader.lines() {
        if Scope::is_interrupted() {
            break;
        }
        match line {
            Ok(line) => {
                i += 1;
                let line = if line_numbers {
                    format!("{:>6}: {}", i, line)
                } else {
                    line
                };
                match mode {
                    Mode::Cat => my_println!("{line}")?,
                    Mode::Head => {
                        if i > lines {
                            break;
                        };
                        my_println!("{line}")?;
                    }
                    Mode::Tail => {
                        if tail.len() == lines {
                            tail.pop_front();
                        }
                        tail.push_back(line);
                    }
                }
            }
            Err(e) => {
                return Err(e.to_string());
            }
        }
    }

    for line in tail {
        my_println!("{line}")?;
    }

    Ok(())
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "cat".to_string(),
        inner: Arc::new(CatHeadTail::new(Mode::Cat)),
    });
    register_command(ShellCommand {
        name: "head".to_string(),
        inner: Arc::new(CatHeadTail::new(Mode::Head)),
    });
    register_command(ShellCommand {
        name: "tail".to_string(),
        inner: Arc::new(CatHeadTail::new(Mode::Tail)),
    });
}
