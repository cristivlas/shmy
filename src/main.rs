use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
mod cmds;
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

        loop {
            if self.interactive {
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
                    let trimmed = buffer.split('#').next().unwrap_or(&buffer).trim_end();
                    if !trimmed.is_empty() {
                        if trimmed.ends_with('\\') {
                            input.push_str(&trimmed[..trimmed.len() - 1]); // Remove backslash
                        } else {
                            input.push_str(&trimmed);
                            if self.interactive {
                                self.eval(&input);
                                input.clear();
                            } else {
                                input.push('\n');
                            }
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
            Ok(v) => {
                println!("{}", &v);
            }
            Err(s) => {
                eprintln!("{}.", s);
            }
        }
    }
}

fn main() -> Result<(), String> {
    let mut shell = parse_cmd_line()?;
    shell.read_input()
}

fn parse_cmd_line() -> Result<Shell, String> {
    let mut shell = Shell {
        source: None,
        interactive: true,
        interp: Interp {},
    };

    let args: Vec<String> = env::args().collect();
    for arg in &args[1..] {
        if arg.starts_with("-") {
            if arg == "-c" {
                shell.interactive = false;
            }
        } else {
            if !shell.interactive {
                let file = File::open(&arg).map_err(|e| format!("{}: {}", arg, e))?;
                shell.source = Some(Box::new(BufReader::new(file)));
            }
        }
    }

    if shell.source.is_none() {
        shell.source = Some(Box::new(BufReader::new(io::stdin())));
    }

    Ok(shell)
}
