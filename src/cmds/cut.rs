use super::{register_command, Exec, Flag, ShellCommand};
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
        let mut flags = CommandFlags::with_help();
        flags.add_value(
            'd',
            "delimiter",
            "Specify the regex delimiter (default: tab)",
        );
        flags.add_value(
            'f',
            "fields",
            "Specify the fields to extract (comma-separated list)",
        );

        Self { flags }
    }

    fn mode_specific_help(&self) -> &str {
        "Extract specific fields or columns from files or standard input using regex delimiters."
    }
}

impl Exec for CutCommand {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse_relaxed(scope, args);

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... [FILE]...", name);
            println!("{}", self.mode_specific_help());
            println!("\nOptions:");
            println!("{}", flags.help());
            println!("Example: ps | cut -d\\s+ -f4,2");
            println!("Split result of 'ps' commands using one or more spaces as delimiter, output colums 4 and 2");
            return Ok(Value::success());
        }

        let delimiter = flags.value("delimiter").unwrap_or("\t");

        let regex_delimiter =
            Regex::new(&delimiter).map_err(|e| format!("Invalid regex delimiter: {}", e))?;

        let fields: Vec<usize> = flags
            .value("fields")
            .ok_or_else(|| "Fields option is required.".to_string())?
            .split(',')
            .map(|s| {
                s.parse::<usize>()
                    .map_err(|e| format!("Invalid field number: {}", e))
            })
            .collect::<Result<Vec<_>, _>>()?;

        if filenames.is_empty() {
            scope.show_eof_hint();
            let mut stdin = BufReader::new(io::stdin());
            process_cut(&mut stdin, &regex_delimiter, &fields)?;
        } else {
            for filename in &filenames {
                let path = Path::new(filename)
                    .dereference()
                    .map_err(|e| format_error(&scope, filename, args, e))?;

                let file =
                    File::open(&path).map_err(|e| format_error(&scope, filename, args, e))?;
                let mut reader = BufReader::new(file);
                process_cut(&mut reader, &regex_delimiter, &fields)?;
            }
        };

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
