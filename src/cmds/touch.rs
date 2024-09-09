use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::utils::format_error;
use crate::{eval::Value, scope::Scope};
use filetime::FileTime;
use std::fs::OpenOptions;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::SystemTime;

struct Touch {
    flags: CommandFlags,
}

impl Touch {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag(
            'c',
            "no-create",
            "Do not create the file if it does not exist",
        );
        flags.add_flag('h', "no-dereference", "Do not follow symbolic links");
        Self { flags }
    }
}

impl Exec for Touch {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let command_args = flags.parse_all(scope, args);

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
        let no_dereference = flags.is_present("no-dereference");

        for filename in command_args.iter() {
            let path = Path::new(filename);

            let target_path = if no_dereference {
                path.to_path_buf()
            } else {
                path.resolve().map_err(|e| {
                    format_error(
                        scope,
                        filename,
                        args,
                        format!("Failed to resolve path: {}", e),
                    )
                })?
            };

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
        inner: Rc::new(Touch::new()),
    });
}
