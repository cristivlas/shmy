use crate::scope::Scope;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone)]
struct Flag {
    short: Option<char>,
    long: String,
    help: String,
    takes_value: bool,
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

    pub fn parse(&mut self, scope: &Arc<Scope>, args: &[String]) -> Result<Vec<String>, String> {
        let mut args_iter = args.iter().enumerate().peekable();
        let mut non_flag_args = Vec::new();

        while let Some((i, arg)) = args_iter.next() {
            self.index = i;
            if arg.starts_with("--") && arg != "--" {
                self.handle_long_flag(scope, arg, &mut args_iter)?;
            } else if arg.starts_with('-') && arg != "-" {
                self.handle_short_flags(scope, arg, &mut args_iter)?;
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

        while let Some((i, arg)) = args_iter.next() {
            self.index = i;
            if arg.starts_with("--") && arg != "--" {
                if !self.handle_long_flag(scope, arg, &mut args_iter).is_ok() {
                    non_flag_args.push(arg.clone());
                }
            } else if arg.starts_with('-') && arg != "-" {
                if !self.handle_short_flags(scope, arg, &mut args_iter).is_ok() {
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
        if let Some(flag) = self.flags.get(flag_name) {
            if flag.takes_value {
                if let Some((i, value)) = args_iter.next() {
                    self.index = i;
                    self.values.insert(flag.long.clone(), value.clone());
                } else {
                    scope.set_err_arg(self.index);
                    return Err(format!("Flag --{} requires a value", flag_name));
                }
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
                    let final_value = if c == '-' {
                        std::iter::once(value)
                            .chain(args_iter.map(|(_, arg)| arg.clone()))
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
