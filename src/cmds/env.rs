use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::rc::Rc;

struct Environ;

impl Exec for Environ {
    fn exec(&self, _: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
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

        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "env".to_string(),
        inner: Rc::new(Environ),
    });
}
