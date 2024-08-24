use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::cmds::get_command;
use crate::eval::{Scope, Value};
use std::rc::Rc;

struct Run {
    flags: CommandFlags,
}

impl Run {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_option('-', "args", "Pass all remaining arguments to COMMAND");
        Self { flags }
    }
}

impl Exec for Run {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut command_args = flags.parse(args)?;

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

        let cmd_name = &command_args[0].clone();

        if let Some(cmd) = get_command(cmd_name) {
            command_args.remove(0);

            if let Some(cmd_flags) = flags.get_option("args") {
                // Pass all args following -- (or --args) to the command.
                command_args.extend(cmd_flags.split_ascii_whitespace().map(String::from));
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
        inner: Rc::new(Run::new()),
    });
}
