use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use open;
use std::rc::Rc;

struct Open {
    flags: CommandFlags,
}

impl Open {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_option('a', "application", "Application to open with");

        Self { flags }
    }
}

impl Exec for Open {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: open [OPTIONS] FILE...");
            println!("Open one or more files or URLs with the default application.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("open: no file or URL specified".to_string());
        }

        let application = flags.option("application");

        for arg in &args {
            let result = if let Some(app) = application {
                open::with(arg, app)
            } else {
                open::that(arg)
            };

            if let Err(e) = result {
                return Err(format!("Failed to open {}: {}", scope.err_path_str(arg), e));
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "open".to_string(),
        inner: Rc::new(Open::new()),
    });
}