use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{
    eval::Interp, eval::Value, scope::Scope, symlnk::SymLink, utils::format_error,
    utils::sync_env_vars,
};
use colored::*;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

struct Evaluate {
    flags: CommandFlags,
}

impl Evaluate {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('x', "export", "Export variables to environment");
        flags.add_flag('s', "source", "Treat the arguments as file paths");

        Self { flags }
    }
}

impl Exec for Evaluate {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let eval_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: eval EXPR...");
            println!("Evaluate each argument as an expression, stopping at the first error.");
            println!("\nOptions:");
            println!("{}", flags.help());
            println!("NOTE: Each expression to be evaluated must to be surrounded by quotes if non-trivial, e.g.");
            println!("    eval --export \"x = 100\"");
            println!("    eval \"x = 1\" \"y = 2\"");
            return Ok(Value::success());
        }

        let export = flags.is_present("export");
        let source = flags.is_present("source");

        let mut interp = Interp::with_env_vars();
        let global_scope = scope.global();

        for arg in &eval_args {
            let input = if source {
                // Treat arg as the name of a source file.
                // Resolve symbolic links (including WSL).
                let path = Path::new(&arg)
                    .dereference()
                    .map_err(|e| format_error(scope, arg, &args, e))?;

                let mut file = File::open(&path).map_err(|e| format_error(scope, arg, &args, e))?;

                let mut source = String::new(); // buffer for script source code

                file.read_to_string(&mut source)
                    .map_err(|e| format_error(scope, arg, &args, e))?;

                interp.set_file(Some(Arc::new(path.to_string_lossy().to_string())));

                source
            } else {
                interp.set_file(None);

                arg.to_owned()
            };

            match interp.eval(&input, Some(Arc::clone(&scope))) {
                Err(e) => {
                    e.show(scope, &input);
                    let err_expr = if scope.use_colors(&std::io::stderr()) {
                        arg.bright_cyan()
                    } else {
                        arg.normal()
                    };
                    return Err(format!("Error evaluating '{}'", err_expr));
                }

                Ok(value) => {
                    let mut command = false;
                    // Did the expression eval result in running a command? Check for errors.
                    if let Value::Stat(status) = &value {
                        if status.is_err() {
                            return Err(status.to_string())
                        }
                        command = true;
                    }

                    if export {
                        // Export variables from the eval scope to the global scope
                        for (key, var) in scope.vars().iter() {
                            if !key.is_special_var() {
                                global_scope.vars_mut().insert(key.clone(), var.clone());
                            }
                        }
                    } else if !command {
                        my_println!("{}", value)?;
                    }
                }
            }
        }

        if export {
            // Synchronize environment with global scope
            sync_env_vars(global_scope);
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "eval".to_string(),
        inner: Arc::new(Evaluate::new()),
    });
}
