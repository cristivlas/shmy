use super::{register_command, RegisteredCommand, Exec};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use clearscreen;
use std::rc::Rc;

struct Clear {
    flags: CommandFlags,
}

impl Clear {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message", false);
        Clear { flags }
    }
}

impl Exec for Clear {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: clear");
            println!("Clear the terminal screen.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::Int(0));
        }

        match clearscreen::clear() {
            Ok(_) => Ok(Value::Int(0)),
            Err(e) => Err(format!("Could not clear screen: {}", e)),
        }
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "clear".to_string(),
        inner: Rc::new(Clear::new()),
    });

    register_command(RegisteredCommand {
        name: "cls".to_string(),
        inner: Rc::new(Clear::new()),
    });
}
