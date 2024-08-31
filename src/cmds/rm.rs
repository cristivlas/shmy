use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::prompt::{confirm, Answer};
use crate::{eval::Value, scope::Scope};
use std::collections::HashSet;
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
        if self.interactive && (path.is_symlink() || path.exists()) {
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

struct Remove {
    flags: CommandFlags,
}

impl Remove {
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
        Self { flags }
    }

    fn remove_file(&self, path: &Path, ctx: &mut Context) -> io::Result<()> {
        if ctx.confirm(&path, format!("Remove {}", path.display()))? == Answer::Yes {
            fs::remove_file(path)
        } else {
            Ok(())
        }
    }

    fn remove(&self, path: &Path, ctx: &mut Context) -> io::Result<()> {
        if path.is_symlink() {
            #[cfg(windows)]
            {
                use crate::utils::win::remove_link;

                if ctx.confirm(&path, format!("Remove {}", path.display()))? == Answer::Yes {
                    remove_link(path)
                } else {
                    Ok(())
                }
            }
            #[cfg(not(windows))]
            {
                self.remove_file(path, ctx)
            }
        } else if path.is_dir() {
            if ctx.recursive && !ctx.interactive {
                // Nuke it, no questions asked
                fs::remove_dir_all(path)
            } else {
                let prompt = format!(
                    "{} is a directory. Delete all of its content recursively",
                    ctx.scope.err_path(path)
                );

                match confirm(prompt, &ctx.scope, ctx.many)? {
                    Answer::Yes => {
                        let interactive = ctx.interactive;
                        let recursive = ctx.recursive;

                        // Save context
                        ctx.interactive = false;
                        ctx.recursive = true;

                        fs::remove_dir_all(path)?;

                        // Restore context
                        ctx.interactive = interactive;
                        ctx.recursive = recursive;
                    }
                    Answer::All => {
                        ctx.interactive = false;
                        ctx.recursive = true;

                        fs::remove_dir_all(path)?;
                    }
                    Answer::Quit => {
                        ctx.quit = true;
                    }
                    _ => {}
                }
                Ok(())
            }
        } else {
            self.remove_file(path, ctx)
        }
    }
}

impl Exec for Remove {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let paths = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: rm [OPTIONS] FILE...");
            println!("Remove (delete) the specified FILE(s).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if paths.is_empty() {
            return Err("Missing operand".to_string());
        }

        let mut ctx = Context {
            interactive: !flags.is_present("force") || flags.is_present("interactive"),
            recursive: flags.is_present("recursive"),
            many: paths.len() > 1,
            quit: false,
            scope: Rc::clone(&scope),
        };

        // Use a set to dedupe inputs, e.g. avoid ```rm *.rs *.rs``` resulting in error.
        let to_remove: HashSet<&String> = HashSet::from_iter(&paths);

        for path in to_remove.iter() {
            match self.remove(&Path::new(path), &mut ctx) {
                Ok(_) => {}
                Err(e) => {
                    return Err(format!("{}: {}", scope.err_path_arg(path, args), e));
                }
            }

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
        inner: Rc::new(Remove::new()),
    });
}
