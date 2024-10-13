use super::{register_command, Exec, Flag, ShellCommand};
use crate::{
    cmds::flags::CommandFlags, eval::Value, scope::Scope, symlnk::SymLink, utils::format_error,
};
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
        let mut flags = CommandFlags::with_help();
        flags.add_flag('n', "number", "Number output lines");
        flags.add_flag('a', "text", "Transcode to ASCII");

        if matches!(mode, Mode::Head | Mode::Tail) {
            flags.add_value(
                'l',
                "lines",
                "number",
                "Specify the number of lines to output",
            );
        }
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

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
        let text_out = flags.is_present("text");

        let lines = flags
            .value("lines")
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|e| format_error(&scope, v, args, e))
            })
            .unwrap_or(Ok(10))?;

        let result = if filenames.is_empty() {
            scope.show_eof_hint();

            let mode = self.mode.clone();
            let mut stdin = BufReader::new(io::stdin());
            process_input(&mut stdin, mode, line_num, text_out, lines)
        } else {
            let mut result = Ok(());
            for filename in &filenames {
                let path = Path::new(filename)
                    .dereference()
                    .map_err(|e| format_error(&scope, filename, args, e))?;

                let mode = self.mode.clone();
                let file =
                    File::open(&path).map_err(|e| format_error(&scope, filename, args, e))?;

                let mut reader = BufReader::new(file);
                result = process_input(&mut reader, mode, line_num, text_out, lines);

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
    text_out: bool,
    lines: usize,
) -> Result<(), String> {
    let mut i = 0;
    let mut tail = VecDeque::new();

    match tail.try_reserve(lines) {
        Ok(_) => {}
        Err(e) => {
            return Err(format!("Memory allocation failed: {}", e));
        }
    }

    for byte_line in reader.split(b'\n') {
        if Scope::is_interrupted() {
            break;
        }
        let byte_line = byte_line.map_err(|e| format!("Error reading line: {}", e))?;

        let line = if text_out {
            // Filter out non-ASCII bytes and collect into a Vec<u8>
            let filtered_bytes: Vec<u8> = byte_line
                .iter()
                .filter(|&&c| c != 0 && c.is_ascii()) // Filter out non-ASCII bytes
                .copied() // Copy u8 values directly
                .collect(); // Collect the filtered bytes into a Vec<u8>
            String::from_utf8(filtered_bytes).map_err(|e| e.to_string())?
        } else {
            String::from_utf8_lossy(&byte_line).to_string()
        };

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
