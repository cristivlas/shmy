use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::cmds::prompt::{confirm, Answer};
use crate::eval::{Scope, Value};
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

struct Rm {
    flags: CommandFlags,
}

impl Rm {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display t;his help message", false);
        flags.add_flag('f', "force", "Delete without prompting", false);
        flags.add_flag(
            'i',
            "interactive",
            "Prompt before deletion (default)",
            false,
        );
        flags.add_flag(
            'r',
            "recursive",
            "Remove directories and their contents recursively",
            false,
        );
        Rm { flags }
    }

    fn remove_file(&self, path: &Path, interactive: bool) -> io::Result<()> {
        if interactive
            && path.exists()
            && confirm(format!("remove {}", path.display()), false)? != Answer::Yes
        {
            return Ok(());
        }

        fs::remove_file(path)
    }

    fn remove_dir(&self, path: &Path, interactive: bool) -> io::Result<()> {
        if interactive
            && path.exists()
            && confirm(format!("remove directory {}", path.display()), false)? != Answer::Yes
        {
            return Ok(());
        }

        fs::remove_dir_all(path)
    }

    fn remove(&self, path: &Path, interactive: bool, recursive: bool) -> io::Result<()> {
        if path.is_dir() {
            if recursive {
                self.remove_dir(path, interactive)?
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("cannot remove '{}': Is a directory", path.display()),
                ));
            }
        } else {
            self.remove_file(path, interactive)?
        }
        Ok(())
    }
}

impl Exec for Rm {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: rm [OPTIONS] FILE...");
            println!("Remove (delete) the specified FILE(s).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("missing operand".to_string());
        }

        let interactive = !flags.is_present("force") || flags.is_present("interactive");
        let recursive = flags.is_present("recursive");

        for arg in args {
            let path = Path::new(&arg);
            self.remove(&path, interactive, recursive)
                .map_err(|e| e.to_string())?;
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "rm".to_string(),
        inner: Rc::new(Rm::new()),
    });
}
