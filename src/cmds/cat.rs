use super::{register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::utils::format_error;
use crate::{cmds::flags::CommandFlags, eval::Value, scope::Scope};
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{atomic::Ordering, Arc};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::runtime::Runtime;
use tokio::time::{sleep, Duration};

#[derive(Clone)]
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

        // Create a Tokio runtime
        let rt = Runtime::new().map_err(|e| format!("Failed to start Tokio runtime: {}", e))?;

        let result = rt.block_on(async {
            if filenames.is_empty() {
                let mode = self.mode.clone();

                let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
                process_input(&mut stdin, mode, line_num, lines).await
            } else {
                let mut result = Ok(());
                for filename in &filenames {
                    let path = Path::new(filename)
                        .resolve()
                        .map_err(|e| format_error(&scope, filename, args, e))?;

                    let mode = self.mode.clone();
                    let file = File::open(&path)
                        .await
                        .map_err(|e| format!("Error opening file: {}", e))?;

                    let mut reader = BufReader::new(file);
                    result = process_input(&mut reader, mode, line_num, lines).await;

                    if result.is_err() {
                        break;
                    }
                }
                result
            }
        });
        rt.shutdown_background();

        result?;
        Ok(Value::success())
    }
}

async fn check_abort() {
    let poll_interval = Duration::from_millis(50);

    loop {
        if crate::ABORT.load(Ordering::SeqCst) {
            return;
        }
        sleep(poll_interval).await;
    }
}

async fn process_input<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    mode: Mode,
    line_numbers: bool,
    lines: usize,
) -> Result<(), String> {
    let mut lines_stream = reader.lines();
    let mut i = 0;
    let mut tail: VecDeque<_> = VecDeque::with_capacity(lines);

    loop {
        tokio::select! {
            _ = check_abort() => {
                return Err("Aborted".into());
            }
            line = lines_stream.next_line() => {
                if crate::INTERRUPT.load(Ordering::SeqCst) {
                    break;
                }
                match line.map_err(|e| e.to_string())? {
                    Some(line) => {
                        i += 1;
                        let line = if line_numbers {
                            format!("{:>6}: {}", i, line)
                        } else {
                            line
                        };
                        match mode {
                            Mode::Cat => { my_println!("{line}")? },
                            Mode::Head => { if i > lines { break }; my_println!("{line}")?; },
                            Mode::Tail => {
                                if tail.len() == lines {
                                    tail.pop_front();
                                }
                                tail.push_back(line);
                            }
                        }
                    },
                    None => break,
                }
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
