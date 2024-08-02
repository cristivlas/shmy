use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::rc::Rc;
struct Clear;

impl Exec for Clear {
    fn exec(&self, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        println!("{}", args.join(" "));
        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "Clear".to_string(),
        inner: Rc::new(Clear),
    });
}
