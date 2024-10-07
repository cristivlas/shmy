use super::{flags::CommandFlags, get_command, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, job::Job, scope::Scope, utils::executable};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

struct Sudo {
    flags: CommandFlags,
}

impl Sudo {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_value(
            '-',
            "args",
            "arg list",
            "Pass all remaining arguments to COMMAND",
        );
        Self { flags }
    }
}

impl Exec for Sudo {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut command_args = flags.parse_relaxed(scope, args);

        if flags.is_present("help") {
            println!("Usage: sudo [OPTIONS] COMMAND [ARGS]...");
            println!("Execute a command with elevated privileges");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if command_args.is_empty() {
            return Err("No command specified".to_string());
        }

        if cfg!(not(debug_assertions)) {
            if !std::io::stdin().is_terminal() {
                return Err("Cannot pipe or redirect input to elevated command".to_string());
            }

            if !std::io::stdout().is_terminal() || !std::io::stderr().is_terminal() {
                return Err("Cannot pipe or redirect output from elevated command".to_string());
            }
        }

        let cmd_name = command_args.remove(0);

        if let Some(additional_args) = flags.value("args") {
            command_args.extend(additional_args.split_whitespace().map(String::from));
        }

        let (executable, parameters) = if let Some(cmd) = get_command(&cmd_name) {
            let cur_dir =
                std::env::current_dir().map_err(|e| format!("Could not get current dir: {}", e))?;

            if cmd.is_external() {
                let path = cmd.path().to_string_lossy().to_string();

                if cmd.is_script() && !path.ends_with(".msc") {
                    (
                        "cmd.exe".to_owned(),
                        format!(
                            "/K cd {} && {} {}",
                            cur_dir.display(),
                            cmd_name,
                            command_args.join(" ")
                        ),
                    )
                } else {
                    (path, command_args.join(" "))
                }
            } else {
                // Internal command, run it by spawning an instance of this shell.
                (
                    executable().map_err(|e| format!("Could not get executable path: {}", e))?,
                    format!("-k {} {}", cmd_name, command_args.join(" ")),
                )
            }
        } else {
            return Err(format!("Command not found: {}", cmd_name));
        };

        Job::new(scope, Path::new(&executable), &[parameters], true)
            .run()
            .map_err(|e| e.to_string())?;

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "sudo".to_string(),
        inner: Arc::new(Sudo::new()),
    });
}
