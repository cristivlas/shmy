use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::{current_dir, eval::Value, scope::Scope};
use std::cell::RefCell;
use std::env;
use std::path::Path;
use std::sync::Arc;

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
        flags.add_flag('?', "help", "Display this help message");
        Self {
            stack: RefCell::new(Vec::new()),
            flags,
        }
    }

    fn do_chdir(&self, scope: &Arc<Scope>, dir: &str) -> Result<(), String> {
        let path = Path::new(dir).resolve().map_err(|e| e.to_string())?;

        env::set_current_dir(&path)
            .map_err(|e| format!("Change dir to \"{}\": {}", scope.err_str(dir), e))?;
        Ok(())
    }

    fn chdir(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let parsed_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            match name {
                "cd" | "chdir" => {
                    println!("Usage: {} [DIR]", name);
                    println!("Change the current directory to DIR.");
                }
                "pushd" => {
                    println!("Usage: pushd <DIR>");
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

            return Ok(Value::success());
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
                self.do_chdir(scope, &new_dir)?
            }
            "pushd" => {
                let new_dir = if parsed_args.is_empty() {
                    return Err("pushd: no directory specified".to_string());
                } else {
                    parsed_args.join(" ")
                };
                self.stack.borrow_mut().push(current_dir()?);
                self.do_chdir(scope, &new_dir)?
            }
            "popd" => {
                if self.stack.borrow().is_empty() {
                    return Err("popd: directory stack empty".to_string());
                }
                let old_dir = self.stack.borrow_mut().pop().unwrap();
                self.do_chdir(scope, &old_dir)?
            }
            _ => unreachable!(),
        }

        Ok(Value::success())
    }
}

impl Exec for ChangeDir {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        self.chdir(name, args, scope)
    }
}

impl PrintWorkingDir {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('h', "help", "Display this help message");
        Self { flags }
    }
}

impl Exec for PrintWorkingDir {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let _ = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: pwd");
            println!("Print the current working directory.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        println!("{}", current_dir()?);
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    let chdir = Arc::new(ChangeDir::new());

    register_command(ShellCommand {
        name: "cd".to_string(),
        inner: Arc::clone(&chdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "pushd".to_string(),
        inner: Arc::clone(&chdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "popd".to_string(),
        inner: Arc::clone(&chdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "pwd".to_string(),
        inner: Arc::new(PrintWorkingDir::new()),
    });
}
