use super::{register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::process;
use std::sync::Arc;

struct Exit;

impl Exec for Exit {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Arc<Scope>) -> Result<Value, String> {
        let exit_code = if args.len() > 0 {
            args[0]
                .parse::<i32>()
                .map_err(|_| "Invalid exit code. Please provide a valid integer.".to_string())?
        } else {
            0
        };

        process::exit(exit_code);
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "exit".to_string(),
        inner: Arc::new(Exit),
    });
}
