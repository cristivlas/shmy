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
            panic!("flag '{}' (or its short form) already exists", long);
        }
    }

    /// Add boolean flag
    pub fn add_flag(&mut self, short: char, long: &str, help: &str) {
        self.add(Some(short), long, false, help);
    }

    /// Add flag that takes a value
    pub fn add_value_flag(&mut self, short: char, long: &str, help: &str) {
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
        for (pos, c) in arg[1..].chars().enumerate() {
            if let Some(flag) = self.flags.values().find(|f| f.short == Some(c)) {
                if flag.takes_value {
                    if pos + 2 < arg.len() {
                        return Err(format!(
                            "Short flag -{} that requires a value should be last in group",
                            c
                        ));
                    } else if let Some(value) = args_iter.next() {
                        // Special case -- consumes all flags
                        let next = if c == '-' {
                            std::iter::once(value.clone())
                                .chain(args_iter.cloned())
                                .collect::<Vec<_>>()
                                .join(" ")
                        } else {
                            value.clone()
                        };
                        self.values.insert(flag.long.clone(), next);
                    } else {
                        return Err(format!("Flag -{} requires a value", c));
                    }
                } else {
                    self.values.insert(flag.long.clone(), "true".to_string());
                }
            } else {
                return Err(format!("Unknown flag: -{}", c));
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn is_present(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    pub fn get_value(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
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
                "{}--{:16}\t{}\n",
                short_flag_help, flag.long, flag.help
            ));
        }
        help_text
    }
}
