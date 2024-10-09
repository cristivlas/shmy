///
/// eval command
/// Named to avoid conflict with the eval.rs file that contains the core expr. evaluation code.
///
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
        flags.add_flag('s', "source", "Treat the argument as path to script source");
        flags.add_flag('q', "quiet", "Quiet (suppress output)");

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
            println!("If --source is specified, the 1st argument after that is assumed to be the path to a");
            println!("file containing script code, and the rest of the arguments are passed to the script.");
            println!();
            println!("Each expression to be evaluated must to be surrounded by quotes if non-trivial, e.g.");
            println!("    eval --export \"x = 100\"");
            println!("    eval \"x = 1\" \"y = 2\"");
            println!();
            println!("Without quotes, the intepreter evaluates the command line as one single expression.");
            println!();
            return Ok(Value::success());
        }

        let export = flags.is_present("export");
        let source = flags.is_present("source");

        let eval_scope = Scope::with_parent_and_hooks(Some(scope.clone()), None);
        let mut interp = Interp::new(scope.clone());

        let mut args_iter = eval_args.iter();

        while let Some(arg) = args_iter.next() {
            let input = if source {
                // Treat arg as the name of a source file.
                // Resolve symbolic links (including WSL).
                let path = Path::new(&arg)
                    .dereference()
                    .map_err(|e| format_error(scope, arg, &args, e))?;

                let mut file = File::open(&path).map_err(|e| format_error(scope, arg, &args, e))?;

                let mut script = String::new(); // buffer for script source code

                file.read_to_string(&mut script)
                    .map_err(|e| format_error(scope, arg, &args, e))?;

                interp.set_file(Some(Arc::new(path.to_string_lossy().to_string())));

                // eval --source treats the 1st arg as a filename, and passes subsequent args to script.
                // Populate $0, $1 etc.
                let mut cmd_args = vec![arg.clone()];
                scope.insert("0".to_string(), Value::from(arg.as_str()));

                let mut n = 0;

                while let Some(next_arg) = args_iter.next() {
                    n += 1;
                    scope.insert(format!("{}", n), Value::from(next_arg.as_str()));
                    cmd_args.push(next_arg.clone());
                }
                scope.insert("#".to_string(), Value::Int(n));
                scope.insert("@".to_string(), Value::from(cmd_args.join(" ").as_str()));

                script
            } else {
                interp.set_file(None);

                arg.to_owned()
            };

            match interp.eval(&input, Some(eval_scope.clone())) {
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
                            return Err(status.clone().err().unwrap().to_string());
                        }
                        command = true;
                    }

                    if export {
                        let global_scope = scope.global();
                        // Export variables from the eval scope to the global scope
                        for (key, var) in eval_scope.vars().iter() {
                            if !key.is_special_var() {
                                global_scope.vars_mut().insert(key.clone(), var.clone());
                            }
                        }
                    } else if !command && !flags.is_present("quiet") {
                        my_println!("{}", value)?;
                    }
                }
            }
        }

        if export {
            // Synchronize environment with global scope
            sync_env_vars(&scope.global());
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
