use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::rc::Rc;

struct Environ {
    flags: CommandFlags,
}

impl Environ {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message", false);
        Environ { flags }
    }
}

impl Exec for Environ {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: vars");
            println!("Display variables.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        // Borrow the vars from scope
        let vars = scope.vars.borrow();

        // Collect keys and sort them
        let mut keys: Vec<String> = vars.keys().cloned().collect();
        keys.sort(); // Sort the keys lexicographically

        // Iterate over sorted keys
        for key in keys {
            if let Some(variable) = vars.get(&key) {
                println!("{}={}", key, variable);
            }
        }

        Ok(Value::success())
    }

    fn is_external(&self) -> bool {
        false
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "vars".to_string(),
        inner: Rc::new(Environ::new()),
    });
}
