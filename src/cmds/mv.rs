use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::prompt::{confirm, Answer};
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct Mv {
    flags: CommandFlags,
}

impl Mv {
    fn new() -> Self {
        let mut flags = CommandFlags::with_follow_links();
        flags.add_flag_enabled('i', "interactive", "Prompt before overwriting files");
        flags.add_alias(Some('f'), "force", "no-interactive");

        Self { flags }
    }

    fn move_file(
        src: &Path,
        dest: &Path,
        interactive: &mut bool,
        one_of_many: bool,
        scope: &Arc<Scope>,
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

    fn get_dest_path(scope: &Arc<Scope>, path: &str) -> Result<PathBuf, String> {
        Ok(PathBuf::from(path)
            .dereference()
            .and_then(|p| Ok(p.into()))
            .unwrap_or(
                Path::new(".")
                    .canonicalize()
                    .map_err(|e| format!("{}: {}", scope.err_str(path), e))?
                    .join(path),
            ))
    }
}

impl Exec for Mv {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
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

        let follow = flags.is_present("follow-links");
        let mut interactive = flags.is_present("interactive");

        let dest = Self::get_dest_path(scope, args.last().unwrap())?;

        let sources = &args[..args.len() - 1];
        let is_batch = sources.len() > 1;

        for src in sources {
            let mut src_path = PathBuf::from(src);
            if follow {
                src_path = src_path
                    .resolve(follow)
                    .map_err(|e| format!("{}: {}", scope.err_str(src), e))?
                    .into();
            }
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
        inner: Arc::new(Mv::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;
    use std::fs::{self, File};
    use tempfile::tempdir;

    #[test]
    fn test_move_file_success() {
        let temp_dir = tempdir().unwrap();
        let src_file = temp_dir.path().join("source.txt");
        let dest_dir = temp_dir.path().join("dest");

        // Create a source file
        File::create(&src_file).unwrap();
        fs::create_dir(&dest_dir).unwrap(); // Create destination directory

        let scope = Scope::new();
        let mut interactive = false;

        // Move file
        let result = Mv::move_file(&src_file, &dest_dir, &mut interactive, false, &scope);
        assert!(result.is_ok());

        // Check that the file was moved
        let final_dest = dest_dir.join("source.txt");
        assert!(final_dest.exists());
        assert!(!src_file.exists()); // Source should no longer exist
    }

    #[test]
    fn test_move_file_same_source_and_dest() {
        let temp_dir = tempdir().unwrap();
        let src_file = temp_dir.path().join("source.txt");

        // Create a source file
        File::create(&src_file).unwrap();

        let scope = Scope::new();
        let mut interactive = false;

        // Attempt to move file to the same location
        let result = Mv::move_file(&src_file, &src_file, &mut interactive, false, &scope);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            format!(
                "{}: Source and destination are the same",
                scope.err_path(&src_file)
            )
        );
    }

    #[test]
    fn test_move_file_to_subdirectory_of_itself() {
        let temp_dir = tempdir().unwrap();
        let src_dir = temp_dir.path().join("source_dir");
        let dest_subdir = src_dir.join("sub_dir");

        // Create source directory and subdirectory
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dest_subdir).unwrap();

        let scope = Scope::new();
        let mut interactive = false;

        // Try to move the directory into its own subdirectory
        let result = Mv::move_file(&src_dir, &dest_subdir, &mut interactive, false, &scope);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap(),
            format!(
                "Cannot move {} to a subdirectory of itself",
                scope.err_path(&src_dir)
            )
        );
    }

    #[test]
    fn test_exec_missing_args() {
        let mv = Mv::new();
        let scope = Scope::new();

        // Test missing source and destination
        let args = vec![];
        let result = mv.exec("mv", &args, &scope);
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), "Missing source and destination");

        // Test missing destination
        let args = vec!["source.txt".to_string()];
        let result = mv.exec("mv", &args, &scope);
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), "Missing destination");
    }
}
