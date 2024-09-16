use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use crossterm::{
    cursor, execute,
    terminal::{Clear, ClearType},
};
use std::sync::Arc;

struct ClearScreen {
    flags: CommandFlags,
}

impl ClearScreen {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }
}

impl Exec for ClearScreen {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: clear");
            println!("Clear the terminal screen.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        execute!(
            std::io::stdout(),
            cursor::MoveTo(0, 0),
            Clear(ClearType::All)
        )
        .map_err(|e| format!("Could not clear screen: {}", e))?;

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "clear".to_string(),
        inner: Arc::new(ClearScreen::new()),
    });

    register_command(ShellCommand {
        name: "cls".to_string(),
        inner: Arc::new(ClearScreen::new()),
    });
}
