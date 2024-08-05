use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use std::fs::File;
use std::io::{self, BufRead};
use std::rc::Rc;

struct Cat;

fn print_file<F: std::io::Read>(file: &mut F, line_numbers: bool) -> Result<(), String> {
    let reader = io::BufReader::new(file);
    for line in reader.lines().flatten().enumerate() {
        if line_numbers {
            println!("{:>6}: {}", line.0, line.1);
        } else {
            println!("{}", line.1);
        }
    }
    Ok(())
}

impl Exec for Cat {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut filenames = Vec::new();
        let mut line_numbers = false;
        for arg in args {
            if arg.starts_with('-') {
                for flag in arg.chars().skip(1) {
                    match flag {
                        'n' => {
                            line_numbers = true;
                        }
                        _ => {
                            Err(format!("Unrecognized command line flag: -{}", flag))?;
                        }
                    }
                }
            } else {
                filenames.push(arg);
            }
        }

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

#[ctor::ctor]
fn register() {
    register_command(BuiltinCommand {
        name: "cat".to_string(),
        inner: Rc::new(Cat),
    });
}
