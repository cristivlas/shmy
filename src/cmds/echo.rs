use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::rc::Rc;
struct Echo;

impl Exec for Echo {
    fn exec(&self, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        println!("{}", args.join(" "));
        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "echo".to_string(),
        inner: Rc::new(Echo),
    });
}
