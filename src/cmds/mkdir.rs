use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::fs;
use std::path::Path;
use std::rc::Rc;

struct Mkdir {
    flags: CommandFlags,
}

impl Mkdir {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('p', "parents", "Create parent directories as needed");
        Mkdir { flags }
    }
}

impl Exec for Mkdir {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: mkdir [OPTIONS] DIRECTORY...");
            println!("Create the DIRECTORY(ies), if they do not already exist.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing directory name".to_string());
        }

        let create_parents = flags.is_present("parents");

        for (i, dir) in args.iter().enumerate() {
            let path = Path::new(dir);
            let result = if create_parents {
                fs::create_dir_all(path)
            } else {
                fs::create_dir(path)
            };
            match result {
                Ok(_) => {}
                Err(e) => {
                    scope.set_err_arg(i);
                    return Err(format!("{}: {}", scope.err_path(path), e));
                }
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    let mkdir = Rc::new(Mkdir::new());

    register_command(ShellCommand {
        name: "md".to_string(),
        inner: Rc::clone(&mkdir) as Rc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "mkdir".to_string(),
        inner: Rc::clone(&mkdir) as Rc<dyn Exec>,
    });
}
