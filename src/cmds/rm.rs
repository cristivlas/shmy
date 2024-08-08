use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::rc::Rc;

struct Rm {
    flags: CommandFlags,
}

impl Rm {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message", false);
        flags.add_flag('i', "interactive", "Prompt before deletion", false);
        flags.add_flag(
            'r',
            "recursive",
            "Remove directories and their contents recursively",
            false,
        );
        Rm { flags }
    }

    fn remove_file(&self, path: &Path, interactive: bool) -> io::Result<()> {
        if interactive && path.exists() {
            print!("rm: remove '{}'? ", path.display());
            io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                return Ok(());
            }
        }

        fs::remove_file(path)
    }

    fn remove_dir(&self, path: &Path, interactive: bool) -> io::Result<()> {
        if interactive && path.exists() {
            print!("rm: remove directory '{}'? ", path.display());
            io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                return Ok(());
            }
        }

        fs::remove_dir_all(path)
    }

    fn remove(&self, path: &Path, interactive: bool, recursive: bool) -> io::Result<()> {
        if path.is_dir() {
            if recursive {
                self.remove_dir(path, interactive)?;
            } else {
                eprintln!("rm: cannot remove '{}': Is a directory", path.display());
            }
        } else {
            self.remove_file(path, interactive)?;
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
            return Ok(Value::Int(0));
        }

        if args.is_empty() {
            return Err("rm: missing operand".to_string());
        }

        let interactive = flags.is_present("interactive");
        let recursive = flags.is_present("recursive");

        for arg in args {
            let path = Path::new(&arg);
            if let Err(e) = self.remove(&path, interactive, recursive) {
                eprintln!("rm: {}", e);
            }
        }

        Ok(Value::Int(0))
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "rm".to_string(),
        inner: Rc::new(Rm::new()),
    });
}
