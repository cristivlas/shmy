use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::rc::Rc;

struct ChangeDir;
struct PrintWorkingDir;

impl Exec for ChangeDir {
    fn exec(&self, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        println!("{}", args.join(" "));
        Ok(Value::Int(0))
    }
}

impl Exec for PrintWorkingDir {
    fn exec(&self, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        println!("{}", args.join(" "));
        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "cd".to_string(),
        inner: Rc::new(ChangeDir),
    });

    register_command(BuiltinCommand {
        name: "pwd".to_string(),
        inner: Rc::new(PrintWorkingDir),
    });
}
