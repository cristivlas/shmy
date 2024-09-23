use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink, utils::format_error};
use open;
use std::path::Path;
use std::sync::Arc;

struct Open {
    flags: CommandFlags,
}

impl Open {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_value('a', "application", "Application to open with");

        Self { flags }
    }
}

impl Exec for Open {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
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

        let application = flags.value("application");

        for arg in &args {
            let path = Path::new(arg)
                .dereference()
                .map_err(|e| format_error(scope, arg, &args, e))?
                .to_path_buf();

            let result = if let Some(app) = application {
                open::with(path, app)
            } else {
                open::that(path)
            };

            result.map_err(|e| format_error(scope, arg, &args, e))?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "open".to_string(),
        inner: Arc::new(Open::new()),
    });
}
