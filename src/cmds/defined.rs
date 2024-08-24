use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::rc::Rc;

struct Defined {
    flags: CommandFlags,
}

impl Defined {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }
}

impl Exec for Defined {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: defined NAME...");
            println!("Check the existence of variable(s) with the given name(s).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }
        for a in args {
            if scope.lookup(&a).is_none() {
                return Err(format!("{} is undefined", a));
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "defined".to_string(),
        inner: Rc::new(Defined::new()),
    });
}
