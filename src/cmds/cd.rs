use super::{register_command, RegisteredCommand, Exec};
use crate::{
    current_dir, debug_print,
    eval::{Scope, Value},
};

use std::cell::RefCell;
use std::env;
use std::rc::Rc;

struct ChangeDir {
    stack: RefCell<Vec<String>>,
}
struct PrintWorkingDir;

impl ChangeDir {
    fn new() -> Self {
        Self {
            stack: RefCell::new(Vec::new()),
        }
    }

    fn chdir(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let new_dir = if args.is_empty() {
            scope
                .lookup_value("HOME")
                .unwrap_or(Value::default())
                .to_string()
        } else {
            args.join(" ")
        };
        debug_print!(&new_dir);

        match env::set_current_dir(&new_dir) {
            Ok(_) => Ok(Value::Int(0)),
            Err(e) => Err(format!("Change dir to \"{}\": {}", &new_dir, e)),
        }
    }
}

impl Exec for ChangeDir {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        if ["cd", "chdir", "pushd"].contains(&name) {
            if name == "pushd" {
                self.stack.borrow_mut().push(current_dir()?);
            }
            self.chdir(name, args, scope)?;
        } else if name == "popd" {
            if self.stack.borrow().is_empty() {
                return Err(format!("Already at the top of the stack"));
            }
            let old_dir = self.stack.borrow_mut().pop().unwrap();
            self.chdir(name, &vec![old_dir], scope)?;
        }

        Ok(Value::Int(0))
    }
}

impl Exec for PrintWorkingDir {
    fn exec(&self, _name: &str, _args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        println!("{}", current_dir()?);
        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    let chdir = Rc::new(ChangeDir::new());

    register_command(RegisteredCommand {
        name: "cd".to_string(),
        inner: Rc::clone(&chdir) as Rc<dyn Exec>,
    });

    register_command(RegisteredCommand {
        name: "pushd".to_string(),
        inner: Rc::clone(&chdir) as Rc<dyn Exec>,
    });

    register_command(RegisteredCommand {
        name: "popd".to_string(),
        inner: Rc::clone(&chdir) as Rc<dyn Exec>,
    });

    register_command(RegisteredCommand {
        name: "pwd".to_string(),
        inner: Rc::new(PrintWorkingDir),
    });
}
