use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::fs::File;
use std::io::{self, Read, Write};
use std::rc::Rc;

struct Cat;

impl Exec for Cat {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        if args.is_empty() {
            // Read from stdin
            let mut stdin = io::stdin();
            let mut buffer = String::new();
            stdin
                .read_to_string(&mut buffer)
                .map_err(|e| e.to_string())?;
            io::stdout()
                .write(buffer.as_bytes())
                .map_err(|e| e.to_string())?;
            io::stdout().flush().map_err(|e| e.to_string())?;
        } else {
            // Read from files specified in args
            for filename in args {
                let mut file = File::open(&filename).map_err(|e| e.to_string())?;
                let mut buffer = String::new();
                file.read_to_string(&mut buffer)
                    .map_err(|e| e.to_string())?;
                println!("{}", buffer);
            }
        }
        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "cat".to_string(),
        inner: Rc::new(Cat),
    });
}
