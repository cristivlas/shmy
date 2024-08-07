use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::fs::File;
use std::io::{self, BufRead};
use std::rc::Rc;

struct Cat {
    flags: CommandFlags,
}

impl Cat {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('n', "number", "Number all output lines", false);
        flags.add_flag('?', "help", "Display this help message", false);
        Cat { flags }
    }
}

impl Exec for Cat {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: cat [OPTION]... [FILE]...");
            println!("Concatenate FILE(s) to standard output.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::Int(0));
        }

        let line_numbers = flags.is_present("number");

        if filenames.is_empty() {
            print_file(&mut io::stdin(), line_numbers)?;
        } else {
            for filename in &filenames {
                let mut file = File::open(&filename).map_err(|e| e.to_string())?;
                print_file(&mut file, line_numbers)?;
            }
        }
        Ok(Value::Int(0))
    }
}

fn print_file<F: std::io::Read>(file: &mut F, line_numbers: bool) -> Result<(), String> {
    let reader = io::BufReader::new(file);
    for (i, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        if line_numbers {
            println!("{:>6}: {}", i + 1, line);
        } else {
            println!("{}", line);
        }
    }
    Ok(())
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "cat".to_string(),
        inner: Rc::new(Cat::new()),
    });
}
