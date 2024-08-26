use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Ident, scope::Scope, scope::Variable};
use std::collections::HashMap;
use std::env;
use std::rc::Rc;

struct Vars {
    flags: CommandFlags,
}

impl Vars {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('l', "local", "Display inner, local scope variables only");
        Vars { flags }
    }

    fn collect_vars(scope: &Rc<Scope>, local_only: bool) -> HashMap<Ident, Variable> {
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
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: vars [-l]");
            println!("Display variables visible in the current scope.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let local_only = flags.is_present("local");

        if !local_only && name == "env" {
            // Print the environment directly.
            let vars: Vec<String> = env::vars().map(|(key, _)| key).collect();

            for key in vars {
                my_println!("{}={}", key, env::var(&key).map_err(|e| e.to_string())?)?;
            }
        } else {
            // Collect variables
            let vars = Self::collect_vars(scope, local_only);

            // Collect keys and sort them
            let mut keys: Vec<Ident> = vars.keys().cloned().collect();
            keys.sort();

            // Iterate over sorted keys
            for key in keys {
                if let Some(variable) = vars.get(&key) {
                    my_println!("{}={}", key, variable)?;
                }
            }
        }
        Ok(Value::success())
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
