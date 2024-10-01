use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use colored::*;
use regex::Regex;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

struct Grep {
    flags: CommandFlags,
}

impl Grep {
    fn new() -> Self {
        let mut flags = CommandFlags::with_follow_links();
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
        flags.add_flag(
            'l',
            "hyperlink",
            "Include hyperlinks to files and lines in the output",
        );
        flags.add_flag('r', "recursive", "Recursively search subdirectories");
        flags.add_flag(
            'v',
            "invert-match",
            "Invert the sense of matching, showing non-matching lines",
        );
        flags.add(
            None,
            "hidden",
            None,
            "Include hidden (starting with a dot) files and directories",
        );
        flags.add_with_default(None, "messages", None, "Show error messages", Some("true"));
        flags.add_alias(Some('s'), "silent", "no-messages");

        Self { flags }
    }

    fn collect_files(
        &self,
        scope: &Arc<Scope>,
        args: &[String], // Original args, for finding bad arg index in case of error
        paths: &[String],
        follow: bool,
        hidden: bool,
        recursive: bool,
        silent: bool,
        visited: &mut HashSet<String>,
    ) -> Vec<PathBuf> {
        // Files to processs
        let mut files = Vec::new();
        for p in paths {
            // Handle Ctrl+C
            if Scope::is_interrupted() {
                return files;
            }

            let path = Path::new(p);

            if path.is_symlink() {
                if follow {
                    match path.dereference() {
                        Ok(path) => files.extend(self.collect_files(
                            scope,
                            args,
                            &[path.to_string_lossy().to_string()],
                            follow,
                            hidden,
                            recursive,
                            silent,
                            visited,
                        )),
                        Err(e) => {
                            if !silent {
                                my_warning!(scope, "Could not resolve {}: {}", scope.err_str(p), e);
                            }
                        }
                    }
                } else {
                    my_warning!(
                        scope,
                        "Omitting symlink (-L/--follow-links option not set): {}",
                        scope.err_str(p)
                    );
                }
            } else if path.is_file() {
                files.push(path.to_path_buf());
            } else if path.is_dir() {
                if recursive {
                    match path.dereference() {
                        Ok(path) => {
                            if !visited.insert(path.to_string_lossy().to_string()) {
                                continue;
                            }
                        }
                        Err(e) => {
                            if !silent {
                                my_warning!(
                                    scope,
                                    "Could not dereference {}: {}",
                                    scope.err_str(p),
                                    e
                                );
                            }
                            continue;
                        }
                    }

                    match fs::read_dir(path) {
                        Err(e) => {
                            if !silent {
                                my_warning!(
                                    scope,
                                    "Could not read directory {}: {}",
                                    scope.err_path(path),
                                    e
                                );
                            }
                            continue;
                        }
                        Ok(dir) => {
                            files.extend(dir.filter_map(Result::ok).flat_map(|entry| {
                                if !hidden && entry.file_name().to_string_lossy().starts_with(".") {
                                    vec![]
                                } else {
                                    self.collect_files(
                                        scope,
                                        args,
                                        &[entry.path().to_string_lossy().to_string()],
                                        follow,
                                        hidden,
                                        recursive,
                                        silent,
                                        visited,
                                    )
                                }
                            }));
                        }
                    }
                } else {
                    my_warning!(
                        scope,
                        "Omitting directory (-r/--recursive option not set): {}",
                        scope.err_str(p)
                    );
                }
            }
        }
        files
    }

    fn process_line(
        filename: Option<&Path>,
        line_number: usize,
        line: &str,
        regex: &Regex,
        line_number_flag: bool,
        ignore_case: bool,
        show_filename: bool,
        use_color: bool,
        use_hyperlink: bool,
        invert_match: bool,
    ) {
        let line_to_check = if ignore_case {
            line.to_lowercase()
        } else {
            line.to_string()
        };

        let matches = regex.is_match(&line_to_check);

        if matches != invert_match {
            let mut output = String::new();

            // Handle hyperlinks and filename output
            if use_hyperlink {
                if let Some(name) = filename {
                    let path = name.canonicalize().unwrap_or_else(|_| name.to_path_buf());
                    let url = Url::from_file_path(path).unwrap();
                    let text = format!("{}:{}", name.display(), line_number + 1);
                    let hyperlink = format!(
                        "\x1B]8;;{}?line={}\x1B\\{}\x1B]8;;\x1B\\",
                        url,
                        line_number + 1,
                        text
                    );
                    output.push_str(&hyperlink);
                }
            } else {
                if show_filename {
                    if let Some(name) = filename {
                        if use_color {
                            output.push_str(&format!("{}:", name.to_string_lossy().magenta()));
                        } else {
                            output.push_str(&format!("{}:", name.to_string_lossy().normal()));
                        }
                    }
                }
                if line_number_flag {
                    output.push_str(&format!("{}:", line_number + 1));
                }
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let grep_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: grep [OPTIONS] PATTERN [FILE]...");
            println!("Search for PATTERN in each FILE (or stdin if no FILE is given).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if grep_args.is_empty() {
            return Err("Missing search pattern".to_string());
        }

        let pattern = &grep_args[0];
        let invert_match = flags.is_present("invert-match");

        let follow = flags.is_present("follow-links");
        let hidden = flags.is_present("hidden");
        let ignore_case = flags.is_present("ignore-case");
        let line_number_flag = flags.is_present("line-number");
        let no_filename = flags.is_present("no-filename");
        let recursive = flags.is_present("recursive");
        let silent = !flags.is_present("messages");
        let use_color = scope.lookup("NO_COLOR").is_none() && std::io::stdout().is_terminal();
        let use_filename = flags.is_present("with-filename");
        let use_hyperlink = flags.is_present("hyperlink");

        let regex = if ignore_case {
            Regex::new(&format!("(?i){}", pattern)).map_err(|e| e.to_string())?
        } else {
            Regex::new(pattern).map_err(|e| e.to_string())?
        };

        let files = &grep_args[1..];

        if files.is_empty() {
            // Read from stdin if no files are provided
            scope.show_eof_hint();
            let reader = io::stdin().lock();
            for (line_number, line) in reader.lines().enumerate() {
                if Scope::is_interrupted() {
                    break;
                }

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
                    use_hyperlink,
                    invert_match,
                );
            }
        } else {
            let mut visited = HashSet::new();
            let files_to_process = self.collect_files(
                scope,
                args,
                files,
                follow,
                hidden,
                recursive,
                silent,
                &mut visited,
            );

            let show_filename = if no_filename {
                false
            } else if use_filename || files_to_process.len() > 1 {
                true
            } else {
                false
            };

            for path in &files_to_process {
                if Scope::is_interrupted() {
                    break;
                }
                match File::open(&path) {
                    Ok(file) => {
                        let reader = BufReader::new(file);
                        for (line_number, line) in reader.lines().enumerate() {
                            if Scope::is_interrupted() {
                                break;
                            }

                            match line {
                                Ok(line) => Self::process_line(
                                    Some(path),
                                    line_number,
                                    &line,
                                    &regex,
                                    line_number_flag,
                                    ignore_case,
                                    show_filename,
                                    use_color,
                                    use_hyperlink,
                                    invert_match,
                                ),
                                Err(e) => {
                                    if !silent {
                                        my_warning!(scope, "{}: {}", scope.err_path(path), e);
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        if !silent {
                            my_warning!(scope, "Could not open {}: {}", scope.err_path(path), e);
                        }
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
        inner: Arc::new(Grep::new()),
    });
}

#[cfg(test)]
mod tests {
    ///
    /// must be run with --test-threads 1
    ///
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::PathBuf;

    struct Cleanup;

    impl Drop for Cleanup {
        fn drop(&mut self) {
            _ = fs::remove_file("test_file.txt");
        }
    }
    fn setup_test_file(content: &str) -> (Cleanup, PathBuf) {
        let path = PathBuf::from("test_file.txt");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "{}", content).unwrap();
        (Cleanup {}, path)
    }

    #[test]
    fn test_grep_basic_functionality() {
        let grep = Grep::new();
        let scope = Scope::new();
        let (_cleanup, test_file) = setup_test_file("Hello World\nThis is a test\nGoodbye World");

        let args = vec![
            "grep".to_string(),
            "test".to_string(),
            test_file.to_string_lossy().to_string(),
        ];
        let result = grep.exec("grep", &args, &scope);

        assert!(result.is_ok());
    }

    #[test]
    fn test_ignore_case_flag() {
        let grep = Grep::new();
        let scope = Scope::new();

        let (_cleanup, test_file) = setup_test_file("hello World\nTHIS IS A TEST\nGoodbye World");

        let args = vec![
            "grep".to_string(),
            "-i".to_string(),
            "hello".to_string(),
            test_file.to_string_lossy().to_string(),
        ];
        let result = grep.exec("grep", &args, &scope);
        assert!(result.is_ok());
    }

    #[test]
    fn test_line_number_flag() {
        let grep = Grep::new();
        let scope = Scope::new();

        let (_cleanup, test_file) = setup_test_file("Line 1\nLine 2\nLine 3");

        let args = vec![
            "grep".to_string(),
            "-n".to_string(),
            "Line".to_string(),
            test_file.to_string_lossy().to_string(),
        ];
        let result = grep.exec("grep", &args, &scope);

        assert!(result.is_ok());
    }

    #[test]
    fn test_recursive_directory_search() {
        let grep = Grep::new();
        let scope = Scope::new();

        // Create a temporary directory with nested files for testing
        let temp_dir = tempfile::tempdir().unwrap();
        let sub_dir = temp_dir.path().join("subdir");
        fs::create_dir(&sub_dir).unwrap();

        // Create a test file in the main directory
        let main_file_path = temp_dir.path().join("test_file.txt");
        let mut main_file = File::create(&main_file_path).unwrap();
        writeln!(main_file, "Hello World").unwrap();

        // Create a test file in the subdirectory
        let sub_file_path = sub_dir.join("nested_test_file.txt");
        let mut sub_file = File::create(&sub_file_path).unwrap();
        writeln!(sub_file, "Nested Hello").unwrap();

        // Execute the grep command on the temporary directory
        let args = vec![
            "grep".to_string(),
            "Hello".to_string(),
            temp_dir.path().to_string_lossy().to_string(),
        ];
        let result = grep.exec("grep", &args, &scope);

        assert!(result.is_ok());
    }

    #[test]
    fn test_silent_mode() {
        let grep = Grep::new();
        let scope = Scope::new();

        let (_cleanup, test_file) = setup_test_file("Test file\nAnother line");

        let args = vec![
            "grep".to_string(),
            "-s".to_string(),
            "missing".to_string(),
            test_file.to_string_lossy().to_string(),
        ];
        let result = grep.exec("grep", &args, &scope);

        assert!(result.is_ok());
    }
}
