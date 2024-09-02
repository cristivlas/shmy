use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::utils::format_error;
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::rc::Rc;

struct WordCount {
    flags: CommandFlags,
}

struct CountResult {
    lines: usize,
    words: usize,
    chars: usize,
    bytes: usize,
}

impl WordCount {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('l', "lines", "Print the newline counts");
        flags.add_flag('w', "words", "Print the word counts");
        flags.add_flag('m', "chars", "Print the character counts");
        flags.add_flag('c', "bytes", "Print the byte counts");
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }

    fn count_file(path: &Path) -> io::Result<CountResult> {
        let file = File::open(path)?;
        let reader = BufReader::new(&file);
        let mut result = CountResult {
            lines: 0,
            words: 0,
            chars: 0,
            bytes: 0,
        };

        for line in reader.lines() {
            let line = line?;
            result.lines += 1;
            result.words += line.split_whitespace().count();
            result.chars += line.chars().count();
        }

        #[cfg(unix)]
        {
            result.bytes = fs::metadata(path)?.len() as _;
        }
        #[cfg(windows)]
        {
            result.bytes = fs::metadata(path)?.file_size() as _;
        }

        Ok(result)
    }

    fn count_stdin() -> io::Result<CountResult> {
        let stdin = io::stdin();
        let reader = stdin.lock();
        let mut result = CountResult {
            lines: 0,
            words: 0,
            chars: 0,
            bytes: 0,
        };

        for line in reader.lines() {
            let line = line?;
            result.lines += 1;
            result.words += line.split_whitespace().count();
            result.chars += line.chars().count();
            result.bytes += line.len();
        }

        Ok(result)
    }

    fn print_result(
        result: &CountResult,
        filename: Option<&str>,
        flags: &CommandFlags,
    ) -> Result<(), String> {
        let mut output = String::new();

        if flags.is_empty() || flags.is_present("lines") {
            output.push_str(&format!("{:10}", result.lines));
        }
        if flags.is_empty() || flags.is_present("words") {
            output.push_str(&format!("{:12}", result.words));
        }
        if flags.is_present("chars") {
            output.push_str(&format!("{:14}", result.chars));
        }
        if flags.is_empty() || flags.is_present("bytes") {
            output.push_str(&format!("{:14}", result.bytes));
        }

        if let Some(name) = filename {
            output.push_str(&format!(" {}", name));
        }

        my_println!("{}", output)?;
        Ok(())
    }
}

impl Exec for WordCount {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: wc [OPTION]... [FILE]...");
            println!("Print newline, word, and byte counts for each FILE, and a total line if more than one FILE is specified.");
            println!("\nIf no FILE is specified, read from standard input.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let mut total = CountResult {
            lines: 0,
            words: 0,
            chars: 0,
            bytes: 0,
        };

        if args.is_empty() {
            // Read from stdin
            match WordCount::count_stdin() {
                Ok(result) => WordCount::print_result(&result, None, &flags)?,
                Err(e) => return Err(format!("Error reading stdin: {}", e)),
            }
        } else {
            for file in &args {
                let path = Path::new(&file)
                    .resolve()
                    .map_err(|e| format_error(scope, file, &args, e))?;

                if path.is_dir() {
                    my_warning!(scope, "{}: Is a directory", scope.err_path(&path));
                    continue;
                }

                if path.is_symlink() {
                    my_warning!(scope, "{}: Is a symbolic link", scope.err_path(&path));
                    continue;
                }

                match WordCount::count_file(&path) {
                    Ok(result) => {
                        WordCount::print_result(&result, Some(&file), &flags)?;
                        total.lines += result.lines;
                        total.words += result.words;
                        total.chars += result.chars;
                        total.bytes += result.bytes;
                    }
                    Err(e) => {
                        my_warning!(scope, "{}: {}", scope.err_path_str(file), e);
                    }
                }
            }

            if args.len() > 1 {
                WordCount::print_result(&total, Some("total"), &flags)?;
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "wc".to_string(),
        inner: Rc::new(WordCount::new()),
    });
}
