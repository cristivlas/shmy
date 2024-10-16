use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::sync::Arc;

struct Reset {
    flags: CommandFlags,
}

impl Reset {
    fn new() -> Self {
        let flags = CommandFlags::with_help();
        Self { flags }
    }
}

impl Exec for Reset {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: {}", name);
            println!("Reset terminal.")
        } else {
            println!("\x1b\x63");
        }
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "reset".to_string(),
        inner: Arc::new(Reset::new()),
    });
}
