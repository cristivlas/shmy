use super::{flags::CommandFlags, get_command, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use std::path::Path;
use std::sync::Arc;

struct Run {
    flags: CommandFlags,
}

impl Run {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('L', "follow-links", "Follow symbolic links");
        flags.add_flag('D', "debug", "Debug (dump) command line arguments");
        flags.add_flag(
            'r',
            "raw",
            "Arguments are passed as a raw string that needs to be tokenized",
        );
        flags.add_value('-', "args", "Pass all remaining arguments to COMMAND");
        flags.add_value(
            'd',
            "delimiter",
            "Specify custom delimiters for tokenizing when '--raw' is specified (default: whitespace)",
        );
        Self { flags }
    }
}

impl Exec for Run {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut command_args = flags.parse_all(scope, args);

        if flags.is_present("help") {
            println!("Usage: run COMMAND [ARGS]...");
            println!("Execute the specified command with its arguments.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }
        if command_args.is_empty() {
            return Err("No command specified".to_string());
        }
        let mut cmd_name = command_args.iter().next().cloned().unwrap();

        if flags.is_present("follow-links") {
            cmd_name = Path::new(&cmd_name)
                .dereference()
                .map_err(|e| e.to_string())?
                .display()
                .to_string();
        }

        if let Some(cmd) = get_command(&cmd_name) {
            command_args.remove(0);

            if let Some(cmd_flags) = flags.value("args") {
                // Pass all args following -- (or --args) to the command.
                command_args.extend(cmd_flags.split_ascii_whitespace().map(String::from));
            }
            if flags.is_present("raw") {
                // Use custom delimiter if specified, otherwise use whitespace
                let delimiters = flags.value("delimiter").unwrap_or(" \t\n\r");
                command_args = command_args
                    .iter()
                    .flat_map(|s| {
                        s.split(|c| delimiters.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                    })
                    .collect();
            }
            if flags.is_present("debug") {
                println!("cmd: \"{}\", args: {:?}", cmd.name(), &command_args);
            }

            return cmd.exec(cmd_name.as_str(), &command_args, scope);
        }

        Err(format!("Command not found: {}", cmd_name))
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "run".to_string(),
        inner: Arc::new(Run::new()),
    });
}
