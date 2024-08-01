use super::{register_command, BuiltinCommand, Exec};
use std::rc::Rc;

struct Echo;

impl Exec for Echo {
    fn exec(&self, args: Vec<String>) -> Result<crate::eval::Value, String> {
        println!("{}", args.join(" "));
        Ok(crate::eval::Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand { name: "echo".to_string(), exec: Rc::new(Echo) });
}
