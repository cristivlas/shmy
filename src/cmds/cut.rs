use super::{register_command, Exec, ShellCommand};
use crate::{
    cmds::flags::CommandFlags, eval::Value, scope::Scope, symlnk::SymLink, utils::format_error,
};
use regex::Regex;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;

struct CutCommand {
    flags: CommandFlags,
}

impl CutCommand {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_option(
            'd',
            "delimiter",
            "Specify the regex delimiter (default: tab)",
        );
        flags.add_option(
            'f',
            "fields",
            "Specify the fields to extract (comma-separated list)",
        );
        flags.add_flag('?', "help", "Display this help message");
        CutCommand { flags }
    }

    fn mode_specific_help(&self) -> &str {
        "Extract specific fields or columns from files or standard input using regex delimiters."
    }
}

impl Exec for CutCommand {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse_all(scope, args);

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... [FILE]...", name);
            println!("{}", self.mode_specific_help());
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let delimiter = flags.option("delimiter").unwrap_or("\t");

        let regex_delimiter =
            Regex::new(&delimiter).map_err(|e| format!("Invalid regex delimiter: {}", e))?;

        let fields: Vec<usize> = flags
            .option("fields")
            .ok_or_else(|| "Fields option is required.".to_string())?
            .split(',')
            .map(|s| {
                s.parse::<usize>()
                    .map_err(|e| format!("Invalid field number: {}", e))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let result = if filenames.is_empty() {
            scope.show_eof_hint();
            let mut stdin = BufReader::new(io::stdin());
            process_cut(&mut stdin, &regex_delimiter, &fields)
        } else {
            let mut result = Ok(());
            for filename in &filenames {
                let path = Path::new(filename)
                    .resolve()
                    .map_err(|e| format_error(&scope, filename, args, e))?;

                let file =
                    File::open(&path).map_err(|e| format_error(&scope, filename, args, e))?;
                let mut reader = BufReader::new(file);
                result = process_cut(&mut reader, &regex_delimiter, &fields);

                if result.is_err() {
                    break;
                }
            }
            result
        };

        result?;
        Ok(Value::success())
    }
}

fn process_cut<R: BufRead>(
    reader: &mut R,
    delimiter: &Regex,
    fields: &[usize],
) -> Result<(), String> {
    for line in reader.lines() {
        if Scope::is_interrupted() {
            break;
        }

        match line {
            Ok(line) => {
                // Use regex to split the line by the delimiter, ignoring leading matches
                let columns: Vec<&str> = delimiter.split(&line.trim_start()).collect();
                let mut selected_fields = Vec::new();

                for &field in fields {
                    if field == 0 || field > columns.len() {
                        return Err(format!("Field index {} is out of range", field));
                    }
                    selected_fields.push(columns[field - 1]);
                }

                // Join selected fields back using the original delimiter regex
                my_println!("{}", selected_fields.join(" "))?;
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "cut".to_string(),
        inner: Arc::new(CutCommand::new()),
    });
}