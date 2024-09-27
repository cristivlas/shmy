use colored::Colorize;

use super::{
    flags::CommandFlags, get_command, register_command, registered_commands, unregister_command,
    Exec, Flag, ShellCommand,
};
use crate::{eval::Value, scope::Scope, utils::format_error};
use std::any::Any;
use std::io;
use std::sync::Arc;

pub struct AliasRunner {
    args: Vec<String>,
    cmd: Option<ShellCommand>,
}

impl AliasRunner {
    fn new(args: Vec<String>) -> Self {
        let cmd = get_command(&args[0]);
        Self { args, cmd }
    }
}

impl Exec for AliasRunner {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        if let Some(cmd) = &self.cmd {
            return cmd.cli_flags();
        }
        Box::new(std::iter::empty())
    }

    /// Execute alias via the "eval" command.
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let eval = get_command("eval").expect("eval command not registered");
        let combined_args: String = self
            .args
            .iter()
            .chain(args.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        eval.exec(name, &vec![combined_args], scope)
    }
}

struct Alias {
    flags: CommandFlags,
}

impl Alias {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('r', "remove", "Remove an existing alias");
        flags.add_flag('l', "list", "List all aliases");

        Self { flags }
    }

    fn add(&self, name: String, args: Vec<String>) -> Result<Value, String> {
        if get_command(&name).is_some() {
            Err(format!("{} already exists", name))
        } else {
            assert!(!args.is_empty());
            register_command(ShellCommand {
                name,
                inner: Arc::new(AliasRunner::new(args)),
            });

            Ok(Value::success())
        }
    }

    fn list(&self) {
        let mut count = 0;

        for name in registered_commands(true) {
            let cmd = get_command(&name).unwrap();

            match cmd
                .inner
                .as_ref()
                .as_any()
                .and_then(|any| any.downcast_ref::<AliasRunner>())
            {
                None => {}
                Some(runner) => {
                    count += 1;
                    println!("{}: {}", name, runner.args.join(" "));
                }
            }
        }
        if count == 0 {
            println!("No aliases found.");
        }
    }

    fn remove(&self, name: &str, scope: &Arc<Scope>, args: &[String]) -> Result<Value, String> {
        match get_command(name) {
            None => Err(format_error(scope, name, args, "alias not found")),
            Some(cmd) => {
                if cmd
                    .inner
                    .as_ref()
                    .as_any()
                    .and_then(|any| any.downcast_ref::<AliasRunner>())
                    .is_some()
                {
                    unregister_command(name);
                    Ok(Value::success())
                } else {
                    Err(format_error(scope, name, args, "not an alias"))
                }
            }
        }
    }
}

impl Exec for Alias {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut parsed_args = flags.parse_relaxed(scope, args);

        if flags.is_present("help") {
            println!("Usage: alias [NAME EXPRESSION] [OPTIONS]");
            println!("Register or deregister aliases (expression shortcuts).");
            println!("\nOptions:");
            println!("{}", flags.help());
            println!();
            println!("Examples:");
            println!("    alias la ls -al");
            println!("    alias --remove la");
            println!("    alias unalias \"alias --remove\"");
            println!();
            println!("Using quotes is recommended when registering aliases.");
            return Ok(Value::success());
        }

        if flags.is_present("list") {
            if parsed_args.is_empty() {
                self.list();
            } else {
                eprintln!("--list (or -l) was specified but other arguments were present.");
                let guess = format!("alias {} \"{}\"", args[0], args[1..].join(" "));
                let guess = if scope.use_colors(&io::stderr()) {
                    guess.bright_cyan()
                } else {
                    guess.normal()
                };

                eprintln!("Did you mean: {}?", guess);
            }
            return Ok(Value::success());
        }

        if flags.is_present("remove") {
            if parsed_args.is_empty() {
                return Err("Please specify an alias to remove".to_string());
            }
            let name = &parsed_args[0];
            return self.remove(&name, scope, args);
        }

        // Register new alias
        if parsed_args.is_empty() {
            return Err("NAME not specified".to_string());
        }

        if parsed_args.len() < 2 {
            return Err("EXPRESSION not specified".to_string());
        }

        let name = parsed_args.remove(0);
        self.add(name, parsed_args)
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "alias".to_string(),
        inner: Arc::new(Alias::new()),
    });
}
