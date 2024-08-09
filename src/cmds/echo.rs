use super::{register_command, Exec, RegisteredCommand};
use crate::eval::{Scope, Value};
use std::rc::Rc;

struct Echo;

impl Exec for Echo {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        println!("{}", args.join(" "));
        Ok(Value::success())
    }

    fn is_external(&self) -> bool {
        false
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "echo".to_string(),
        inner: Rc::new(Echo),
    });
}
