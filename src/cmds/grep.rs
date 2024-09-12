use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use colored::*;
use regex::Regex;
use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

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
        flags.add_flag('L', "follow-links", "Follow symbolic links");
        Self { flags }
    }

    fn collect_files(
        &self,
        scope: &Arc<Scope>,
        args: &[String], // Original args, for finding bad arg index in case of error
        paths: &[String],
        follow: bool,
        recursive: bool,
        visited: &mut HashSet<PathBuf>,
    ) -> Vec<PathBuf> {
        // Files to processs
        let mut files = Vec::new();
        for p in paths {
            // Handle Ctrl+C
            if scope.is_interrupted() {
                return files;
            }

            let path = PathBuf::from(p);

            if !visited.insert(path.clone()) {
                continue;
            }

            if path.is_dir() {
                if recursive {
                    files.extend(
                        fs::read_dir(&path)
                            .unwrap()
                            .filter_map(Result::ok)
                            .flat_map(|entry| {
                                self.collect_files(
                                    scope,
                                    args,
                                    &[entry.path().display().to_string()],
                                    follow,
                                    recursive,
                                    visited,
                                )
                            }),
                    );
                } else {
                    my_warning!(
                        scope,
                        "Omitting directory (-r/--recursive option not set): {}",
                        scope.err_str(p)
                    );
                }
            } else if path.is_symlink() {
                if follow {
                    match path.resolve() {
                        Ok(path) => files.extend(self.collect_files(
                            scope,
                            args,
                            &[path.display().to_string()],
                            follow,
                            recursive,
                            visited,
                        )),
                        Err(e) => {
                            my_warning!(scope, "Could not resolve {}: {}", scope.err_str(p), e)
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
                files.push(path);
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
    ) {
        let line_to_check = if ignore_case {
            line.to_lowercase()
        } else {
            line.to_string()
        };

        if regex.is_match(&line_to_check) {
            let mut output = String::new();

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
                        output.push_str(&format!("{}:", name.display()));
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

        let follow = flags.is_present("follow-links");
        let ignore_case = flags.is_present("ignore-case");
        let line_number_flag = flags.is_present("line-number");
        let no_filename = flags.is_present("no-filename");
        let recursive = flags.is_present("recursive");
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
            let stdin = io::stdin();
            let reader = stdin.lock();
            for (line_number, line) in reader.lines().enumerate() {
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
                );
            }
        } else {
            let mut visited = HashSet::new();
            let files_to_process =
                self.collect_files(scope, args, files, follow, recursive, &mut visited);

            let show_filename = if no_filename {
                false
            } else if use_filename || files_to_process.len() > 1 {
                true
            } else {
                false
            };

            for path in &files_to_process {
                if scope.is_interrupted() {
                    break;
                }

                match &fs::read_to_string(&path) {
                    Ok(content) => {
                        for (line_number, line) in content.lines().enumerate() {
                            Self::process_line(
                                Some(path),
                                line_number,
                                line,
                                &regex,
                                line_number_flag,
                                ignore_case,
                                show_filename,
                                use_color,
                                use_hyperlink,
                            );
                        }
                    }
                    Err(e) => my_warning!(scope, "Could not read {}: {}", scope.err_path(path), e),
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
