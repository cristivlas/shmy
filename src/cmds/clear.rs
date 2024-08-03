use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use clearscreen;
use std::rc::Rc;

struct Clear;

impl Exec for Clear {
    fn exec(&self, _name: &str, _args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        match clearscreen::clear() {
            Ok(_) => Ok(Value::Int(0)),
            Err(e) => Err(format!("Could not clear screen: {}", e)),
        }
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "clear".to_string(),
        inner: Rc::new(Clear),
    });

    register_command(BuiltinCommand {
        name: "cls".to_string(),
        inner: Rc::new(Clear),
    });
}
