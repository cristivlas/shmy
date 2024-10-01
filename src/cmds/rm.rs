use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::prompt::{confirm, Answer};
use crate::{eval::Value, scope::Scope, symlnk::SymLink, utils::format_error};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;

struct Context {
    interactive: bool,
    recursive: bool,
    many: bool,
    quit: bool,
    scope: Arc<Scope>,
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
        let mut flags = CommandFlags::with_follow_links();
        flags.add_flag_enabled('i', "interactive", "Prompt before deletion");
        flags.add_alias(Some('f'), "force", "no-interactive");
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let paths = flags.parse_relaxed(scope, args);

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
            interactive: flags.is_present("interactive"),
            recursive: flags.is_present("recursive"),
            many: paths.len() > 1,
            quit: false,
            scope: Arc::clone(&scope),
        };

        let follow_links = flags.is_present("follow-links");

        // Use a set to dedupe inputs, e.g. avoid ```rm *.rs *.rs``` resulting in error.
        let to_remove: HashSet<&String> = HashSet::from_iter(&paths);

        for &path in to_remove.iter() {
            Path::new(path)
                .resolve(follow_links)
                .and_then(|path| self.remove(&path, &mut ctx))
                .map_err(|e| format_error(scope, path, args, e))?;

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
        inner: Arc::new(Remove::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::sync::Arc;

    // Create a test instance of Scope directly if it has a public constructor.
    fn create_test_scope() -> Arc<Scope> {
        let scope = Scope::new();

        scope.insert("NO_COLOR".to_string(), Value::Int(1));
        scope.insert("NO_CONFIRM".to_string(), Value::Int(1));

        scope
    }

    #[test]
    fn test_remove_file() {
        let temp_dir = std::env::temp_dir().join("test_remove_file");
        fs::create_dir_all(&temp_dir).unwrap();
        let file_path = temp_dir.join("test_file.txt");
        File::create(&file_path).unwrap(); // Create the file

        let scope = create_test_scope();
        let remove_cmd = Remove::new();

        let mut ctx = Context {
            interactive: true,
            recursive: false,
            many: false,
            quit: false,
            scope: Arc::clone(&scope),
        };

        // Test removing the file
        assert!(remove_cmd.remove_file(&file_path, &mut ctx).is_ok());
        assert!(!file_path.exists()); // Ensure the file is deleted

        // Clean up
        fs::remove_dir_all(temp_dir).unwrap();
    }

    #[test]
    fn test_remove_directory() {
        let temp_dir = std::env::temp_dir().join("test_remove_dir");
        fs::create_dir_all(&temp_dir).unwrap();
        let dir_path = temp_dir.join("test_dir");
        fs::create_dir_all(&dir_path).unwrap(); // Create a directory

        let scope = create_test_scope();
        let remove_cmd = Remove::new();

        let mut ctx = Context {
            interactive: true,
            recursive: true,
            many: false,
            quit: false,
            scope: Arc::clone(&scope),
        };

        // Test removing the directory
        assert!(remove_cmd.remove(&dir_path, &mut ctx).is_ok());
        assert!(!dir_path.exists()); // Ensure the directory is deleted

        // Clean up
        fs::remove_dir_all(temp_dir).unwrap();
    }
}
