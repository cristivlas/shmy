use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::sync::Arc;

struct Defined {
    flags: CommandFlags,
}

impl Defined {
    fn new() -> Self {
        let flags = CommandFlags::with_help();
        Self { flags }
    }
}

impl Exec for Defined {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: defined NAME...");
            println!("Check the existence of variable(s) with the given name(s).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }
        for a in args {
            if scope.lookup(&a).is_none() {
                return Err(format!("{} is undefined", a));
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "defined".to_string(),
        inner: Arc::new(Defined::new()),
    });
}
