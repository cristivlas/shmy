use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::rc::Rc;
use std::sync::Arc;

struct Clear {
    flags: CommandFlags,
}

impl Clear {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }
}

impl Exec for Clear {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: clear");
            println!("Clear the terminal screen.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        match clearscreen::clear() {
            Ok(_) => Ok(Value::success()),
            Err(e) => Err(format!("Could not clear screen: {}", e)),
        }
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "clear".to_string(),
        inner: Rc::new(Clear::new()),
    });

    register_command(ShellCommand {
        name: "cls".to_string(),
        inner: Rc::new(Clear::new()),
    });
}
