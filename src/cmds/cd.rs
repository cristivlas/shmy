use super::{register_command, Exec, RegisteredCommand};
use crate::{
    cmds::flags::CommandFlags,
    current_dir, debug_print,
    eval::{Scope, Value},
};

use std::cell::RefCell;
use std::env;
use std::rc::Rc;

struct ChangeDir {
    stack: RefCell<Vec<String>>,
    flags: CommandFlags,
}

struct PrintWorkingDir {
    flags: CommandFlags,
}

impl ChangeDir {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message", false);
        Self {
            stack: RefCell::new(Vec::new()),
            flags,
        }
    }

    fn chdir(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let parsed_args = flags.parse(args)?;

        if flags.is_present("help") {
            match name {
                "cd" | "chdir" => {
                    println!("Usage: {} [DIR]", name);
                    println!("Change the current directory to DIR.");
                }
                "pushd" => {
                    println!("Usage: pushd [DIR]");
                    println!("Push the current directory onto the stack and change to DIR.");
                }
                "popd" => {
                    println!("Usage: popd");
                    println!("Pop the top directory from the stack and change to it.");
                }
                _ => unreachable!(),
            }
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::Int(0));
        }

        match name {
            "cd" | "chdir" => {
                let new_dir = if parsed_args.is_empty() {
                    scope
                        .lookup_value("HOME")
                        .unwrap_or(Value::default())
                        .to_string()
                } else {
                    parsed_args.join(" ")
                };
                debug_print!(&new_dir);
                env::set_current_dir(&new_dir)
                    .map_err(|e| format!("Change dir to \"{}\": {}", &new_dir, e))?;
            }
            "pushd" => {
                let new_dir = if parsed_args.is_empty() {
                    return Err("pushd: no directory specified".to_string());
                } else {
                    parsed_args.join(" ")
                };
                self.stack.borrow_mut().push(current_dir()?);
                env::set_current_dir(&new_dir)
                    .map_err(|e| format!("Change dir to \"{}\": {}", &new_dir, e))?;
            }
            "popd" => {
                if self.stack.borrow().is_empty() {
                    return Err("popd: directory stack empty".to_string());
                }
                let old_dir = self.stack.borrow_mut().pop().unwrap();
                env::set_current_dir(&old_dir)
                    .map_err(|e| format!("Change dir to \"{}\": {}", &old_dir, e))?;
            }
            _ => unreachable!(),
        }

        Ok(Value::Int(0))
    }
}

impl Exec for ChangeDir {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        self.chdir(name, args, scope)
    }

    fn is_external(&self) -> bool {
        false
    }
}

impl PrintWorkingDir {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('h', "help", "Display this help message", false);
        Self { flags }
    }
}

impl Exec for PrintWorkingDir {
    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let _ = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: pwd");
            println!("Print the current working directory.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::Int(0));
        }

        println!("{}", current_dir()?);
        Ok(Value::Int(0))
    }

    fn is_external(&self) -> bool {
        false
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
        inner: Rc::new(PrintWorkingDir::new()),
    });
}
