use super::{register_command, Exec, ShellCommand};
use crate::{
    cmds::flags::CommandFlags, eval::Value, scope::Scope, symlnk::SymLink, utils::format_error,
};
use memmap2::Mmap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

struct StringsCommand {
    flags: CommandFlags,
}

impl StringsCommand {
    fn new() -> Self {
        let mut flags = CommandFlags::with_follow_links();
        flags.add_value(
            'n',
            "min-length",
            "Specify the minimum length of strings to output",
        );
        StringsCommand { flags }
    }

    fn mode_specific_help(&self) -> &str {
        "Output printable strings from files."
    }
}

impl Exec for StringsCommand {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... [FILE]...", name);
            println!("{}", self.mode_specific_help());
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if filenames.is_empty() {
            return Err("No file specified".to_string());
        }

        let follow = flags.is_present("follow-links");

        let min_length = flags
            .value("min-length")
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|e| format_error(&scope, v, args, e))
            })
            .unwrap_or(Ok(4))?; // default min-length is 4 (same as Linux)

        for filename in &filenames {
            let mmap = Path::new(filename)
                .resolve(follow)
                .and_then(|path| File::open(&path).and_then(|file| unsafe { Mmap::map(&file) }))
                .map_err(|e| format_error(&scope, filename, args, e))?;

            process_strings(&mmap, min_length)?;
        }

        Ok(Value::success())
    }
}

fn process_strings<R: AsRef<[u8]>>(data: R, min_length: usize) -> Result<(), String> {
    let bytes = data.as_ref();
    let mut current_string = Vec::new();

    for &byte in bytes {
        if byte.is_ascii_alphanumeric() && !byte.is_ascii_whitespace() {
            current_string.push(byte);
        } else if !current_string.is_empty() {
            if current_string.len() >= min_length {
                if let Ok(s) = String::from_utf8(current_string.clone()) {
                    if !s.trim().is_empty() {
                        my_println!("{}", s)?;
                    }
                }
            }
            current_string.clear();
        }
    }

    // Check the last collected string
    if !current_string.is_empty() {
        if current_string.len() >= min_length {
            if let Ok(s) = String::from_utf8(current_string) {
                my_println!("{}", s)?;
            }
        }
    }

    Ok(())
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "strings".to_string(),
        inner: Arc::new(StringsCommand::new()),
    });
}
