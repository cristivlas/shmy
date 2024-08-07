use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::rc::Rc;

struct Echo {
    flags: CommandFlags,
}

impl Echo {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message", false);
        Echo { flags }
    }
}

impl Exec for Echo {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: echo [EXPRESSION]...");
            println!("Evaluate and display the EXPRESSION(s) to standard output.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::Int(0));
        }

        println!("{}", args.join(" "));
        Ok(Value::Int(0))
    }

    fn is_external(&self) -> bool {
        false
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "echo".to_string(),
        inner: Rc::new(Echo::new()),
    });
}
