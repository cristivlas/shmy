use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope};
use crossterm::{
    cursor, execute,
    terminal::{Clear, ClearType},
};
use std::io::{stdout, Write};
use std::sync::Arc;

struct ClearScreen {
    flags: CommandFlags,
}

impl ClearScreen {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('k', "keep", "Keep the scroll (history) buffer");

        Self { flags }
    }
}

impl Exec for ClearScreen {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

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

        let mut stdout = stdout().lock();

        execute!(stdout, cursor::MoveTo(0, 0), Clear(ClearType::All))
            .and_then(|_| {
                if !flags.is_present("keep") {
                    execute!(stdout, Clear(ClearType::Purge))
                } else {
                    Ok(())
                }
                .and_then(|_| stdout.flush())
            })
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
