use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::path::Path;
use std::rc::Rc;

struct Basename {
    flags: CommandFlags,
}

impl Basename {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        Basename { flags }
    }
}

impl Exec for Basename {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: basename [OPTION]... [NAME]...");
            println!("Print the base name of each FILE.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("No arguments provided".to_string());
        }

        for arg in args {
            let path = Path::new(arg);
            let base = path
                .file_name()
                .ok_or_else(|| "Failed to get file name".to_string())?;

            my_println!("{}", base.to_string_lossy())?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "basename".to_string(),
        inner: Rc::new(Basename::new()),
    });
}
