use std::collections::HashMap;

#[derive(Clone)]
struct Flag {
    short: Option<char>,
    long: String,
    help: String,
    takes_value: bool,
}

#[derive(Clone)]
pub struct CommandFlags {
    flags: HashMap<String, Flag>,
    values: HashMap<String, String>,
}

impl CommandFlags {
    pub fn new() -> Self {
        CommandFlags {
            flags: HashMap::new(),
            values: HashMap::new(),
        }
    }

    pub fn add(&mut self, short: Option<char>, long: &str, takes_value: bool, help: &str) {
        if (short.is_some() && self.flags.values().find(|f| f.short == short).is_some())
            || self
                .flags
                .insert(
                    long.to_string(),
                    Flag {
                        short,
                        long: long.to_string(),
                        help: help.to_string(),
                        takes_value: takes_value,
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

    /// Add flag that takes a value
    pub fn add_option(&mut self, short: char, long: &str, help: &str) {
        self.add(Some(short), long, true, help);
    }

    pub fn parse(&mut self, args: &[String]) -> Result<Vec<String>, String> {
        let mut args_iter = args.iter().peekable();
        let mut non_flag_args = Vec::new();

        while let Some(arg) = args_iter.next() {
            if arg.starts_with("--") && arg != "--" {
                self.handle_long_flag(arg, &mut args_iter)?;
            } else if arg.starts_with('-') && arg != "-" {
                self.handle_short_flags(arg, &mut args_iter)?;
            } else {
                non_flag_args.push(arg.clone());
            }
        }

        Ok(non_flag_args)
    }

    /// Parse flags ignoring unrecognized flags.
    /// Useful when command needs to process arguments containing dashes, e.g. ```chmod a-w```
    pub fn parse_all(&mut self, args: &[String]) -> Vec<String> {
        let mut args_iter = args.iter().peekable();
        let mut non_flag_args = Vec::new();

        while let Some(arg) = args_iter.next() {
            if arg.starts_with("--") && arg != "--" {
                if !self.handle_long_flag(arg, &mut args_iter).is_ok() {
                    non_flag_args.push(arg.clone());
                }
            } else if arg.starts_with('-') && arg != "-" {
                if !self.handle_short_flags(arg, &mut args_iter).is_ok() {
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
        arg: &str,
        args_iter: &mut std::iter::Peekable<std::slice::Iter<String>>,
    ) -> Result<(), String> {
        let flag_name = &arg[2..];
        if let Some(flag) = self.flags.get(flag_name) {
            if flag.takes_value {
                if let Some(value) = args_iter.next() {
                    self.values.insert(flag.long.clone(), value.clone());
                } else {
                    return Err(format!("Flag --{} requires a value", flag_name));
                }
            } else {
                self.values.insert(flag.long.clone(), "true".to_string());
            }
        } else {
            return Err(format!("Unknown flag: {}", arg));
        }
        Ok(())
    }

    fn handle_short_flags(
        &mut self,
        arg: &str,
        args_iter: &mut std::iter::Peekable<std::slice::Iter<String>>,
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
                    } else if let Some(next_arg) = args_iter.next() {
                        // Case: -d 2
                        next_arg.clone()
                    } else {
                        return Err(format!("Flag -{} requires a value", c));
                    };
                    // Special case -- consumes all flags
                    let final_value = if c == '-' {
                        std::iter::once(value)
                            .chain(args_iter.cloned())
                            .collect::<Vec<_>>()
                            .join(" ")
                    } else {
                        value
                    };

                    self.values.insert(flag.long.clone(), final_value);
                    break; // Exit the loop as we've consumed the rest of the argument
                } else {
                    self.values.insert(flag.long.clone(), "true".to_string());
                }
            } else {
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

    pub fn get_option(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(|s| s.as_str())
    }

    pub fn help(&self) -> String {
        let mut help_text = String::new();

        for flag in self.flags.values() {
            let short_flag_help = if let Some(short) = flag.short {
                format!("-{}, ", short)
            } else {
                String::new()
            };
            help_text.push_str(&format!(
                "{:4}--{:20} {}\n",
                short_flag_help, flag.long, flag.help
            ));
        }
        help_text
    }
}
