use crate::scope::Scope;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
struct Flag {
    short: Option<char>,
    long: String,
    help: String,
    takes_value: bool,
    default_value: Option<String>,
}

#[derive(Clone)]
pub struct CommandFlags {
    flags: BTreeMap<String, Flag>,
    values: BTreeMap<String, String>,
    index: usize,
}

type ArgsIter<'a> = std::iter::Peekable<std::iter::Enumerate<std::slice::Iter<'a, String>>>;

impl CommandFlags {
    pub fn new() -> Self {
        CommandFlags {
            flags: BTreeMap::new(),
            values: BTreeMap::new(),
            index: 0,
        }
    }

    pub fn add(&mut self, short: Option<char>, long: &str, takes_value: bool, help: &str) {
        self.add_with_default(short, long, takes_value, help, None)
    }

    pub fn add_with_default(
        &mut self,
        short: Option<char>,
        long: &str,
        takes_value: bool,
        help: &str,
        default_value: Option<&str>,
    ) {
        if (short.is_some() && self.flags.values().find(|f| f.short == short).is_some())
            || self
                .flags
                .insert(
                    long.to_string(),
                    Flag {
                        short,
                        long: long.to_string(),
                        help: help.to_string(),
                        takes_value,
                        default_value: default_value.map(String::from),
                    },
                )
                .is_some()
        {
            panic!("flag {} (or its short form) already exists", long);
        }
    }

    /// Add boolean flag
    pub fn add_flag(&mut self, short: char, long: &str, help: &str) {
        self.add(Some(short), long, false, help);
    }

    pub fn add_flag_enabled(&mut self, short: char, long: &str, help: &str) {
        self.add_with_default(Some(short), long, false, help, Some("true"));
    }

    /// Add flag that takes a value
    pub fn add_option(&mut self, short: char, long: &str, help: &str) {
        self.add(Some(short), long, true, help);
    }

    /// Parse command-line arguments and categorize them into flags and non-flag arguments.
    ///
    // Parameters:
    /// - `scope`: The current execution scope, wrapped in an `Arc` for future-proof thread-safety.
    /// - `args`: A slice of strings representing the command-line arguments to be parsed.
    ///
    /// Returns:
    /// - A `Result` containing a vector of non-flag arguments if parsing is successful,
    ///   or an error message as a string if parsing fails.
    pub fn parse(&mut self, scope: &Arc<Scope>, args: &[String]) -> Result<Vec<String>, String> {
        let mut args_iter = args.iter().enumerate().peekable();
        let mut non_flag_args = Vec::new();

        while let Some((i, arg)) = args_iter.next() {
            self.index = i;
            if arg.starts_with("--") && arg != "--" {
                self.handle_long_flag(scope, arg, &mut args_iter)?;
            } else if arg.starts_with('-') {
                if arg != "-" {
                    self.handle_short_flags(scope, arg, &mut args_iter)?;
                }
            } else {
                non_flag_args.push(arg.clone());
            }
        }

        Ok(non_flag_args)
    }

    /// Parse flags ignoring unrecognized flags.
    /// Useful when command needs to process arguments containing dashes, e.g. ```chmod a-w```
    /// and when passing commands to `run` and `sudo`.
    pub fn parse_all(&mut self, scope: &Arc<Scope>, args: &[String]) -> Vec<String> {
        let mut args_iter = args.iter().enumerate().peekable();
        let mut non_flag_args = Vec::new();
        let mut encountered_double_dash = false;

        while let Some((i, arg)) = args_iter.next() {
            self.index = i;
            if encountered_double_dash || arg == "--" {
                encountered_double_dash = true;
                non_flag_args.push(arg.clone());
            } else if arg.starts_with("--") {
                if self.handle_long_flag(scope, arg, &mut args_iter).is_err() {
                    non_flag_args.push(arg.clone());
                }
            } else if arg.starts_with('-') && arg != "-" {
                if self.handle_short_flags(scope, arg, &mut args_iter).is_err() {
                    non_flag_args.push(arg.clone());
                }
            } else {
                non_flag_args.push(arg.clone());
            }
        }

        non_flag_args
    }

    fn handle_long_flag(
        &mut self,
        scope: &Arc<Scope>,
        arg: &str,
        args_iter: &mut ArgsIter,
    ) -> Result<(), String> {
        let flag_name = &arg[2..];
        let is_negation = flag_name.starts_with("no-");
        let actual_flag_name = if is_negation {
            &flag_name[3..]
        } else {
            flag_name
        };

        if let Some(flag) = self.flags.get(actual_flag_name) {
            if flag.takes_value {
                if is_negation {
                    scope.set_err_arg(self.index);
                    return Err(format!(
                        "Flag --no-{} is not valid for option that takes a value",
                        actual_flag_name
                    ));
                }
                if let Some((i, value)) = args_iter.next() {
                    self.index = i;
                    self.values.insert(flag.long.clone(), value.clone());
                } else {
                    scope.set_err_arg(self.index);
                    return Err(format!("Flag --{} requires a value", flag_name));
                }
            } else if is_negation {
                self.values.remove(&flag.long);
            } else {
                self.values.insert(flag.long.clone(), "true".to_string());
            }
        } else {
            scope.set_err_arg(self.index);
            return Err(format!("Unknown flag: {}", arg));
        }
        Ok(())
    }

    fn handle_short_flags(
        &mut self,
        scope: &Arc<Scope>,
        arg: &str,
        args_iter: &mut ArgsIter,
    ) -> Result<(), String> {
        let chars: Vec<char> = arg[1..].chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if let Some(flag) = self.flags.values().find(|f| f.short == Some(c)) {
                if flag.takes_value {
                    let value = if i + 1 < chars.len() {
                        // Case: -d2
                        chars[i + 1..].iter().collect::<String>()
                    } else if let Some((i, next_arg)) = args_iter.next() {
                        // Case: -d 2
                        self.index = i;
                        next_arg.clone()
                    } else {
                        scope.set_err_arg(self.index);
                        return Err(format!("Flag -{} requires a value", c));
                    };
                    // Special case -- consumes all flags
                    let value = if c == '-' {
                        std::iter::once(value)
                            .chain(args_iter.map(|(_, arg)| arg.clone()))
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        value
                    };

                    self.values.insert(flag.long.clone(), value);
                    break; // Exit the loop as we've consumed the rest of the argument
                } else {
                    self.values.insert(flag.long.clone(), "true".to_string());
                }
            } else {
                scope.set_err_arg(self.index);
                return Err(format!("Unknown flag: -{}", c));
            }
            i += 1;
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn is_present(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn option(&self, name: &str) -> Option<&str> {
        self.values
            .get(name)
            .or(self.flags.get(name).and_then(|f| f.default_value.as_ref()))
            .map(|s| s.as_str())
    }
    pub fn help(&self) -> String {
        let mut help_text = String::new();

        for flag in self.flags.values() {
            let short_flag_help = if let Some(short) = flag.short {
                format!("-{}, ", short)
            } else {
                String::new()
            };
            let default_value_help = if let Some(ref default) = flag.default_value {
                format!(" (default: {})", default)
            } else {
                String::new()
            };
            help_text.push_str(&format!(
                "{:4}--{:20} {}{}\n",
                short_flag_help, flag.long, flag.help, default_value_help
            ));
        }
        help_text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn create_test_flags() -> CommandFlags {
        let mut flags = CommandFlags::new();
        flags.add_flag('v', "verbose", "Enable verbose output");
        flags.add_option('o', "output", "Specify output file");
        flags.add_with_default(Some('d'), "debug", true, "Set debug level", Some("0"));
        flags
    }

    #[test]
    fn test_default_values() {
        let flags = create_test_flags();
        assert_eq!(flags.option("verbose"), None);
        assert_eq!(flags.option("debug"), Some("0"));
        assert_eq!(flags.option("output"), None);
    }

    #[test]
    fn test_parse_long_flags() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "--verbose".to_string(),
            "--output".to_string(),
            "file.txt".to_string(),
        ];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
    }

    #[test]
    fn test_parse_short_flags() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["-v".to_string(), "-o".to_string(), "file.txt".to_string()];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
    }

    #[test]
    fn test_boolean_flag_negation() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["--no-verbose".to_string()];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert!(!flags.is_present("verbose"));
    }

    #[test]
    fn test_boolean_flag_negation_after() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["--verbose".to_string(), "--no-verbose".to_string()];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert!(!flags.is_present("verbose"));

        let args = vec![
            "--verbose".to_string(),
            "--no-verbose".to_string(),
            "--verbose".to_string(),
        ];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert!(flags.is_present("verbose"));
    }

    #[test]
    fn test_invalid_negation() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["--no-output".to_string(), "file.txt".to_string()];
        let result = flags.parse(&scope, &args);
        assert!(result.is_err());
    }

    #[test]
    fn test_override_default_value() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["--debug".to_string(), "2".to_string()];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert_eq!(flags.option("debug"), Some("2"));
    }

    #[test]
    fn test_help_output() {
        let flags = create_test_flags();
        let help_text = flags.help();
        assert!(help_text.contains("Enable verbose output"));
        assert!(help_text.contains("Specify output file"));
        assert!(help_text.contains("Set debug level (default: 0)"));
    }

    #[test]
    fn test_is_present() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["--verbose".to_string()];
        let result = flags.parse(&scope, &args);
        assert!(result.is_ok());
        assert!(flags.is_present("verbose"));
        assert!(!flags.is_present("output"));
    }

    #[test]
    fn test_parse_all() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "--verbose".to_string(),
            "--unknown".to_string(),
            "--output".to_string(),
            "file.txt".to_string(),
        ];
        let non_flag_args = flags.parse_all(&scope, &args);
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
        assert_eq!(non_flag_args, vec!["--unknown"]);
    }

    #[test]
    fn test_short_flag_with_separate_value() {
        let mut flags = CommandFlags::new();
        let scope = Arc::new(Scope::new());

        // Adding flags for the test
        flags.add_option('d', "debug", "Set debug level");

        // Test input: '-d' followed by '2' as a separate argument
        let args = vec!["-d".to_string(), "2".to_string()];

        // The parser should successfully parse the flag '-d' and assign the value '2'
        let result: Result<Vec<String>, String> = flags.parse(&scope, &args);

        assert!(result.is_ok(), "Expected the parser to succeed");
        assert_eq!(flags.option("debug"), Some("2"));
    }

    #[test]
    fn test_concatenated_short_flag_with_value() {
        let mut flags = CommandFlags::new();
        let scope = Arc::new(Scope::new());

        // Adding flags for the test
        flags.add_option('d', "debug", "Set debug level");

        // Test input: '-d2', where '2' is concatenated with the flag '-d'
        let args = vec!["-d2".to_string()];

        // The parser should successfully parse the flag '-d' and assign the value '2'
        let result = flags.parse(&scope, &args);

        assert!(result.is_ok(), "Expected the parser to succeed");
        assert_eq!(flags.option("debug"), Some("2"));
    }

    #[test]
    fn test_parse_all_valid_flags() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "--verbose".to_string(),
            "--output".to_string(),
            "file.txt".to_string(),
            "--debug".to_string(),
            "2".to_string(),
        ];
        let non_flag_args = flags.parse_all(&scope, &args);

        assert!(non_flag_args.is_empty(), "Expected no non-flag arguments");
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
        assert_eq!(flags.option("debug"), Some("2"));
    }

    #[test]
    fn test_parse_all_with_unknown_flags() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "--verbose".to_string(),
            "--unknown".to_string(),
            "-x".to_string(),
            "--output".to_string(),
            "file.txt".to_string(),
        ];
        let non_flag_args = flags.parse_all(&scope, &args);

        assert_eq!(non_flag_args, vec!["--unknown", "-x"]);
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
    }

    #[test]
    fn test_parse_all_with_missing_value() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec!["--output".to_string()];
        let non_flag_args = flags.parse_all(&scope, &args);

        assert_eq!(non_flag_args, vec!["--output"]);
        assert!(!flags.is_present("output"));
    }

    #[test]
    fn test_parse_all_with_mixed_valid_and_invalid() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "--verbose".to_string(),
            "--unknown".to_string(),
            "-o".to_string(),
            "file.txt".to_string(),
            "-x".to_string(),
            "--debug".to_string(),
            "2".to_string(),
            "non-flag-arg".to_string(),
        ];
        let non_flag_args = flags.parse_all(&scope, &args);

        assert_eq!(non_flag_args, vec!["--unknown", "-x", "non-flag-arg"]);
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
        assert_eq!(flags.option("debug"), Some("2"));
    }

    #[test]
    fn test_parse_all_with_double_dash() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "--verbose".to_string(),
            "--".to_string(),
            "--output".to_string(),
            "file.txt".to_string(),
        ];
        let non_flag_args = flags.parse_all(&scope, &args);

        assert_eq!(non_flag_args, vec!["--", "--output", "file.txt"]);
        assert!(flags.is_present("verbose"));
        assert!(!flags.is_present("output"));
    }

    #[test]
    fn test_parse_all_with_short_flags() {
        let mut flags = create_test_flags();
        let scope = Arc::new(Scope::new());
        let args = vec![
            "-v".to_string(),
            "-o".to_string(),
            "file.txt".to_string(),
            "-d2".to_string(),
        ];
        let non_flag_args = flags.parse_all(&scope, &args);

        assert!(non_flag_args.is_empty(), "Expected no non-flag arguments");
        assert!(flags.is_present("verbose"));
        assert_eq!(flags.option("output"), Some("file.txt"));
        assert_eq!(flags.option("debug"), Some("2"));
    }
}
