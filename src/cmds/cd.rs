use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::env;
use std::rc::Rc;

struct ChangeDir;
struct PrintWorkingDir;

impl Exec for ChangeDir {
    fn exec(&self, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        if args.len() != 1 {
            Err("Exactly one argument expected")?;
        }
        let new_dir = &args[0];
        match env::set_current_dir(&new_dir) {
            Ok(_) => Ok(Value::Int(0)),
            Err(e) => Err(format!("Error changing directory: {}", e)),
        }
    }
}

impl Exec for PrintWorkingDir {
    fn exec(&self, _args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        match env::current_dir() {
            Ok(path) => println!("{}", path.to_path_buf().to_string_lossy()),
            Err(e) => return Err(format!("Error getting current directory: {}", e)),
        }
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
