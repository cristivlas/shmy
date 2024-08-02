use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Write};
mod cmds;
#[macro_use]
mod eval;
use eval::Interp;

struct Shell {
    source: Option<Box<dyn BufRead>>,
    interactive: bool,
    interp: Interp,
}

impl Shell {
    fn read_input(&mut self) -> Result<(), String> {
        if let Some(reader) = self.source.take() {
            self.read_lines(reader)
        } else {
            Err("Input source is unexpectedly None".to_string())
        }
    }

    fn read_lines<R: BufRead>(&mut self, mut reader: R) -> Result<(), String> {
        let mut buffer = String::new();
        let mut input: String = String::new();
        let mut escape = false;

        loop {
            if !escape && self.interactive {
                print!("mysh> ");
                io::stdout().flush().unwrap();
            }
            buffer.clear();
            match reader.read_line(&mut buffer) {
                Ok(0) => {
                    // End of input
                    if !input.is_empty() {
                        self.eval(&input);
                    }
                    break;
                }
                Ok(_) => {
                    let trimmed = buffer.trim_end();

                    if trimmed.is_empty() {
                        input.push('\n'); // Keep newlines for correct Location info.
                    } else if trimmed.ends_with('\\') {
                        escape = true;
                        input.push_str(&trimmed[..trimmed.len() - 1]); // Remove backslash
                    } else {
                        escape = false;
                        input.push_str(&trimmed);
                        if self.interactive {
                            self.eval(&input);
                            input.clear();
                        } else {
                            input.push('\n');
                        }
                    }

                    buffer.clear();
                }
                Err(e) => return Err(format!("Failed to read input: {}", e)),
            }
        }

        Ok(())
    }

    fn eval(&mut self, input: &String) {
        match self.interp.eval(input) {
            Ok(result) => {
                debug_print!(&result);
            }
            Err(s) => {
                eprintln!("{}.", s);
            }
        }
    }
}

fn main() -> Result<(), ()> {
    match &mut parse_cmd_line() {
        Err(e) => {
            eprint!("Command line error: {}.", e);
        }
        Ok(shell) => match shell.read_input() {
            Err(e) => eprintln!("{}.", e),
            _ => {}
        },
    }
    Ok(())
}

fn parse_cmd_line() -> Result<Shell, String> {
    let mut shell = Shell {
        source: None,
        interactive: true,
        interp: Interp {},
    };

    let args: Vec<String> = env::args().collect();
    for (i, arg) in args.iter().enumerate().skip(1) {
        if arg.starts_with("-") {
            if arg == "-c" {
                if !shell.interactive {
                    Err("cannot specify -c command and scripts at the same time")?;
                }
                shell.source = Some(Box::new(Cursor::new(format!(
                    "{}",
                    args[i + 1..].join(" ")
                ))));
                shell.interactive = false;
                break;
            }
        } else {
            let file = File::open(&arg).map_err(|e| format!("{}: {}", arg, e))?;
            shell.source = Some(Box::new(BufReader::new(file)));
            shell.interactive = false;
        }
    }

    if shell.source.is_none() {
        shell.source = Some(Box::new(BufReader::new(io::stdin())));
    }

    Ok(shell)
}
