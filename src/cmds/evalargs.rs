use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Interp, Scope, Value};
use crate::utils::sync_env_vars;
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

        let export = flags.is_present("export");
        let source = flags.is_present("source");

        let mut interp = Interp::new();
        let eval_scope = Some(Rc::clone(&scope));
        let global_scope = scope.global();

        for arg in &args {
            let input = if source {
                // Treat arg as the name of a source file.
                let mut file = File::open(&arg)
                    .map_err(|e| format!("Could not open {}: {}", scope.err_path_str(&arg), e))?;

                let mut source = String::new();
                file.read_to_string(&mut source)
                    .map_err(|e| format!("Could not read {}: {}", scope.err_path_str(&arg), e))?;

                interp.set_file(Some(Rc::new(arg.to_string())));

                source
            } else {
                interp.set_file(None);

                arg.to_owned()
            };

            match interp.eval(&input, eval_scope.to_owned()) {
                Err(e) => {
                    e.show(&input);
                    return Err(format!("Error evaluating '{}'", scope.err_path_str(&arg)));
                }

                Ok(value) => {
                    if export {
                        // Export variables from the eval scope to the global scope
                        for (key, var) in scope.vars.borrow().iter() {
                            if !key.is_special_var() {
                                global_scope.insert(key.to_string(), var.value());
                            }
                        }
                    } else {
                        my_println!("{}", value)?;
                    }
                }
            }
        }

        // Synchronize environment with global scope
        sync_env_vars(&global_scope);

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
