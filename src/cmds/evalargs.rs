use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Interp, Scope, Value};
use std::borrow::Borrow;
use std::fs::File;
use std::io::Read;
use std::rc::Rc;

struct Evaluate {
    flags: CommandFlags,
}

impl Evaluate {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('x', "export", "Export variables to environment");
        flags.add_flag('s', "source", "Treat the arguments as file paths");

        Self { flags }
    }
}

impl Exec for Evaluate {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: eval EXPR...");
            println!("Evaluate each argument as an expression, stopping at the first error.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let mut interp = Interp::new();
        let eval_scope = Some(Rc::clone(&scope));
        let global_scope = scope.global();

        for arg in args {
            let input = if flags.is_present("source") {
                // Treat arg as the name of a source file
                let mut file = File::open(&arg)
                    .map_err(|e| format!("Could not open {}: {}", scope.err_path_str(&arg), e))?;

                let mut source = String::new();
                file.read_to_string(&mut source)
                    .map_err(|e| format!("Could not read {}: {}", scope.err_path_str(&arg), e))?;

                interp.set_file(Some(Rc::new(arg)));

                source
            } else {
                interp.set_file(None);

                arg.to_owned()
            };

            // TODO: error handling

            let value = interp
                .eval(&input, eval_scope.to_owned())
                .map_err(|e| e.to_string())?;

            if flags.is_present("export") {
                for (key, var) in scope.vars.borrow().iter() {
                    // TODO: is_special_var()

                    if matches!(key.borrow(), "__errors" | "__stderr" | "__stdout") {
                        continue;
                    }

                    global_scope.insert(key.to_string(), var.value());

                    std::env::set_var(key.to_string(), var.to_string());
                }
            } else {
                my_println!("{}", value)?;
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "eval".to_string(),
        inner: Rc::new(Evaluate::new()),
    });
}
