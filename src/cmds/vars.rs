use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value, Variable};
use std::collections::HashMap;
use std::rc::Rc;

struct Vars {
    flags: CommandFlags,
}

impl Vars {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('l', "local", "Display only variables in the current scope");
        Vars { flags }
    }

    fn collect_vars(scope: &Rc<Scope>, local_only: bool) -> HashMap<String, Variable> {
        let mut all_vars = HashMap::new();
        let mut current_scope = Some(Rc::clone(scope));

        while let Some(scope) = current_scope {
            for (key, value) in scope.vars.borrow().iter() {
                if !all_vars.contains_key(key) {
                    all_vars.insert(key.clone(), value.clone());
                }
            }
            if local_only {
                break;
            }
            current_scope = scope.parent.as_ref().map(Rc::clone);
        }

        all_vars
    }
}

impl Exec for Vars {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: vars [-l]");
            println!("Display variables visible in the current scope.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let local_only = flags.is_present("local");

        // Collect variables
        let vars = Self::collect_vars(scope, local_only);

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
    let vars = Rc::new(Vars::new());

    register_command(ShellCommand {
        name: "env".to_string(),
        inner: Rc::clone(&vars) as Rc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "vars".to_string(),
        inner: Rc::clone(&vars) as Rc<dyn Exec>,
    });
}
