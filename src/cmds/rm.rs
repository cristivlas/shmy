use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::prompt::{confirm, Answer};
use std::fs;
use std::io;
use std::path::Path;
use std::rc::Rc;

struct Context {
    interactive: bool,
    recursive: bool,
    many: bool,
    quit: bool,
    scope: Rc<Scope>,
}

impl Context {
    fn confirm(&mut self, path: &Path, prompt: String) -> io::Result<Answer> {
        if self.interactive && path.exists() {
            match confirm(prompt, &self.scope, self.many)? {
                Answer::All => {
                    self.interactive = false;
                    return Ok(Answer::Yes);
                }
                Answer::Quit => {
                    self.quit = true;
                    return Ok(Answer::No);
                }
                Answer::No => return Ok(Answer::No),
                Answer::Yes => return Ok(Answer::Yes),
            }
        }

        Ok(Answer::Yes)
    }
}

struct Rm {
    flags: CommandFlags,
}

impl Rm {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('f', "force", "Delete without prompting");
        flags.add_flag('i', "interactive", "Prompt before deletion (default)");
        flags.add_flag(
            'r',
            "recursive",
            "Remove directories and their contents recursively",
        );
        Rm { flags }
    }

    fn remove_file(&self, path: &Path, ctx: &mut Context) -> io::Result<()> {
        if ctx.confirm(&path, format!("Remove {}", path.display()))? == Answer::Yes {
            fs::remove_file(path)
        } else {
            Ok(())
        }
    }

    fn remove_dir(&self, path: &Path, ctx: &mut Context) -> io::Result<()> {
        if ctx.confirm(&path, format!("Remove directory {}", path.display()))? == Answer::Yes {
            fs::remove_dir_all(path)
        } else {
            Ok(())
        }
    }

    fn remove(&self, path: &Path, ctx: &mut Context) -> io::Result<()> {
        if path.is_dir() {
            if ctx.recursive {
                self.remove_dir(path, ctx)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("Cannot remove '{}': Is a directory", path.display()),
                ))
            }
        } else {
            self.remove_file(path, ctx)
        }
    }
}

impl Exec for Rm {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
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

        let mut ctx = Context {
            interactive: !flags.is_present("force") || flags.is_present("interactive"),
            recursive: flags.is_present("recursive"),
            many: args.len() > 1,
            quit: false,
            scope: Rc::clone(&scope),
        };

        for arg in args {
            let path = Path::new(&arg);
            self.remove(&path, &mut ctx).map_err(|e| e.to_string())?;
            if ctx.quit {
                break;
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "rm".to_string(),
        inner: Rc::new(Rm::new()),
    });
}
