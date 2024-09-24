use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink, utils::format_error};
use filetime::FileTime;
use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

struct Touch {
    flags: CommandFlags,
}

impl Touch {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag(
            'c',
            "no-create",
            "Do not create the file if it does not exist",
        );
        Self { flags }
    }
}

impl Exec for Touch {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let command_args = flags.parse_relaxed(scope, args);

        if flags.is_present("help") {
            println!("Usage: touch [OPTIONS] FILE...");
            println!("Update the access and modification times of each FILE to the current time.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if command_args.is_empty() {
            return Err("No file specified".to_string());
        }

        let no_create = flags.is_present("no-create");

        for filename in command_args.iter() {
            let target_path = Path::new(filename)
                .dereference()
                .map_err(|e| {
                    format_error(
                        scope,
                        filename,
                        args,
                        format!("Failed to resolve path: {}", e),
                    )
                })?
                .to_path_buf();

            if target_path.exists() {
                // Update the last access and modification times
                let now = FileTime::from_system_time(SystemTime::now());
                filetime::set_file_times(&target_path, now, now).map_err(|e| {
                    format_error(
                        scope,
                        filename,
                        args,
                        format!("Failed to update time: {}", e),
                    )
                })?;
            } else if !no_create {
                // Create the file if it doesn't exist and -c is not specified
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .open(&target_path)
                    .map_err(|e| {
                        format_error(
                            scope,
                            filename,
                            args,
                            format!("Failed to create file: {}", e),
                        )
                    })?;
            } else {
                my_warning!(
                    scope,
                    "{} does not exist and was not created (due to -c option).",
                    scope.err_path_arg(filename, args)
                );
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "touch".to_string(),
        inner: Arc::new(Touch::new()),
    });
}
