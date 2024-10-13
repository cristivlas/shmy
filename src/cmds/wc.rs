use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::utils::format_error;
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

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
        let mut flags = CommandFlags::with_help();
        flags.add_flag('l', "lines", "Print the newline counts");
        flags.add_flag('w', "words", "Print the word counts");
        flags.add_flag('m', "chars", "Print the character counts");
        flags.add_flag('c', "bytes", "Print the byte counts");

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

        for byte_line in reader.split(b'\n') {
            let byte_line = byte_line?;
            let line = String::from_utf8_lossy(&byte_line);
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

    fn count_stdin(scope: &Arc<Scope>) -> io::Result<CountResult> {
        scope.show_eof_hint();
        let reader = io::stdin().lock();
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
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
            match WordCount::count_stdin(scope) {
                Ok(result) => WordCount::print_result(&result, None, &flags)?,
                Err(e) => return Err(format!("Error reading stdin: {}", e)),
            }
        } else {
            for file in &args {
                let path = Path::new(&file)
                    .dereference()
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
                        my_warning!(scope, "{}: {}", scope.err_str(file), e);
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
        inner: Arc::new(WordCount::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    // RAII struct for managing the lifecycle of a test file.
    struct TestFile {
        path: PathBuf,
    }

    impl TestFile {
        fn new(content: &str) -> Self {
            let path = PathBuf::from("test_file.txt");
            let mut file = File::create(&path).expect("Failed to create test file");
            file.write_all(content.as_bytes())
                .expect("Failed to write to test file");
            TestFile { path }
        }

        fn path(&self) -> &PathBuf {
            &self.path
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            // Automatically clean up the file when the struct goes out of scope.
            std::fs::remove_file(&self.path).expect("Failed to remove test file");
        }
    }

    #[test]
    fn test_count_file() {
        let test_file = TestFile::new("Hello world\nThis is a test\n");
        let result = WordCount::count_file(test_file.path()).unwrap();
        assert_eq!(result.lines, 2);
        assert_eq!(result.words, 6);
        assert_eq!(result.chars, 25);
        assert_eq!(result.bytes, 27); // Assuming UTF-8 encoding, include newlines
    }
}
