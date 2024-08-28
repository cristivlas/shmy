use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::path::Path;
use std::rc::Rc;

struct Realpath {
    flags: CommandFlags,
}

impl Realpath {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }
}

impl Exec for Realpath {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: realpath [OPTION]... [FILE]...");
            println!("Print the canonicalized absolute path of each FILE.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("No arguments provided".to_string());
        }

        for (i, arg) in args.iter().enumerate() {
            scope.set_err_arg(i);
            let path = Path::new(arg);
            let canonical_path = path
                .canonicalize()
                .map_err(|e| format!("{}: {}", scope.err_path(path), e))?;

            my_println!("{}", canonical_path.display())?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "realpath".to_string(),
        inner: Rc::new(Realpath::new()),
    });
}
