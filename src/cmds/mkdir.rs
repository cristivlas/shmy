use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use std::fs;
use std::path::Path;
use std::sync::Arc;

struct Mkdir {
    flags: CommandFlags,
}

impl Mkdir {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('p', "parents", "Create parent directories as needed");

        Self { flags }
    }
}

impl Exec for Mkdir {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: mkdir [OPTIONS] DIRECTORY...");
            println!("Create the DIRECTORY(ies), if they do not already exist.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing directory name".to_string());
        }

        let create_parents = flags.is_present("parents");

        for (i, dir) in args.iter().enumerate() {
            Path::new(dir)
                .dereference()
                .and_then(|path| {
                    if create_parents {
                        fs::create_dir_all(path)
                    } else {
                        fs::create_dir(path)
                    }
                })
                .map_err(|e| {
                    scope.set_err_arg(i);
                    format!("{}: {}", scope.err_path_arg(dir, &args), e)
                })?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    let mkdir = Arc::new(Mkdir::new());

    register_command(ShellCommand {
        name: "md".to_string(),
        inner: Arc::clone(&mkdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "mkdir".to_string(),
        inner: Arc::clone(&mkdir) as Arc<dyn Exec>,
    });
}
