use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::prompt::{confirm, Answer};
use crate::{eval::Value, scope::Scope};
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

        Self { flags }
    }

    fn move_file(
        src: &Path,
        dest: &Path,
        interactive: &mut bool,
        one_of_many: bool,
        scope: &Rc<Scope>,
    ) -> Result<bool, String> {
        let final_dest = if dest.is_dir() {
            dest.join(
                src.file_name()
                    .ok_or(format!("Invalid source filename: {}", scope.err_path(src)))?,
            )
        } else {
            dest.to_path_buf()
        };

        if src == final_dest {
            return Err(format!(
                "{}: Source and destination are the same",
                scope.err_path(src)
            ));
        }
        if final_dest.starts_with(src) {
            return Err(format!(
                "Cannot move {} to a subdirectory of itself",
                scope.err_path(src)
            ));
        }

        if final_dest.exists() && *interactive {
            match confirm(
                format!("Overwrite {}", final_dest.display()),
                scope,
                one_of_many,
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

        fs::rename(&src, &final_dest).map_err(|error| {
            format!(
                "Failed to move or rename {} to {}: {}",
                scope.err_path(src),
                scope.err_path(final_dest.as_path()),
                error
            )
        })?;

        Ok(true) // Continue with next file, if any
    }

    fn get_dest_path(scope: &Rc<Scope>, path: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(path).canonicalize().unwrap_or(
            Path::new(".")
                .canonicalize()
                .map_err(|e| format!("{}: {}", scope.err_path_str(path), e))?
                .join(path),
        ))
    }
}

impl Exec for Mv {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

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

        let dest = Self::get_dest_path(scope, args.last().unwrap())?;

        let sources = &args[..args.len() - 1];
        let is_batch = sources.len() > 1;

        for src in sources {
            let src_path = Path::new(src)
                .canonicalize()
                .map_err(|e| format!("Cannot canonicalize: {}: {}", src, e))?;

            if !Self::move_file(&src_path, &dest, &mut interactive, is_batch, scope)? {
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
