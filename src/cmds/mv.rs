use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::prompt::{confirm, Answer};
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct Mv {
    flags: CommandFlags,
}

impl Mv {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('f', "force", "Do not prompt before overwriting");
        flags.add_flag('i', "interactive", "Prompt before overwriting files");
        Mv { flags }
    }

    fn move_file(
        src: &Path,
        dest: &Path,
        interactive: &mut bool,
        batch: bool,
        scope: &Rc<Scope>,
    ) -> Result<bool, String> {
        let mut final_dest = if dest.is_dir() {
            dest.join(
                src.file_name()
                    .ok_or(format!("Invalid source filename: '{}'", src.display()))?,
            )
        } else {
            dest.to_path_buf()
        };
        final_dest = final_dest.canonicalize().unwrap_or(final_dest);

        if src == final_dest {
            return Err(format!("'{}': Source and destination are the same", src.display()));
        }

        if final_dest.exists() && *interactive {
            match confirm(
                format!("Overwrite '{}'", final_dest.display()),
                scope,
                batch,
            )
            .map_err(|e| e.to_string())?
            {
                Answer::Yes => {}
                Answer::No => return Ok(true), // Continue with next file
                Answer::All => {
                    *interactive = false;
                }
                Answer::Quit => return Ok(false), // Stop processing files
            }
        }

        fs::rename(&src, &final_dest).map_err(|e| {
            format!(
                "Failed to move or rename '{}' to '{}': {}",
                src.display(),
                final_dest.display(),
                e
            )
        })?;

        Ok(true) // Continue with next file, if any
    }
}

impl Exec for Mv {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: mv [OPTIONS] SOURCE... DEST");
            println!("Move (rename) SOURCE(s) to DESTination.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("Missing source and destination".to_string());
        }
        if args.len() < 2 {
            return Err("Missing destination".to_string());
        }

        let mut interactive = !flags.is_present("force") || flags.is_present("interactive");
        let dest = PathBuf::from(args.last().unwrap());
        let sources = &args[..args.len() - 1];

        let batch = sources.len() > 1;

        for src in sources {
            let src_path = Path::new(src)
                .canonicalize()
                .map_err(|e| format!("Cannot canonicalize: '{}': {}", src, e))?;

            if !Mv::move_file(&src_path, &dest, &mut interactive, batch, scope)? {
                break; // Stop if move_file returns false (user chose to quit)
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "mv".to_string(),
        inner: Rc::new(Mv::new()),
    });
}
