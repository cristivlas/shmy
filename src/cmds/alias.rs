use colored::Colorize;

use super::{
    flags::CommandFlags, get_command, register_command, registered_commands, unregister_command,
    Exec, Flag, ShellCommand,
};
use crate::{eval::Value, prompt::confirm, prompt::Answer, scope::Scope, utils::format_error};
use std::any::Any;
use std::io;
use std::sync::Arc;

pub struct AliasRunner {
    pub args: Vec<String>,
    cmd: Option<ShellCommand>,
    overriden: Option<ShellCommand>,
}

impl AliasRunner {
    fn new(args: Vec<String>, overriden: Option<ShellCommand>) -> Self {
        let arg = args[0].split_ascii_whitespace().collect::<Vec<_>>()[0];
        let cmd = get_command(arg);
        Self {
            args,
            cmd,
            overriden,
        }
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
        // Concatenate registered alias args with command line args wrapped in raw strings.
        let expr = format!(
            "{} {}",
            self.args.join(" "),
            args.iter()
                .map(|s| format!("r\"({})\"", s))
                .collect::<Vec<_>>()
                .join(" ")
        );

        eval.exec(name, &vec![expr], scope)
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

    fn add(
        &self,
        name: String,
        args: Vec<String>,
        scope: Option<Arc<Scope>>,
    ) -> Result<Value, String> {
        assert!(!args.is_empty());

        let existing = get_command(&name);
        if existing.is_some() && scope.is_some() {
            if confirm(
                format!("Command '{}' already defined, override", name),
                &scope.unwrap(),
                false,
            )
            .ok()
                != Some(Answer::Yes)
            {
                return Ok(Value::success());
            }
        }

        register_command(ShellCommand {
            name,
            inner: Arc::new(AliasRunner::new(args, existing)),
        });

        Ok(Value::success())
    }

    fn register(&self, name: &str, args: &[&str]) -> Result<Value, String> {
        self.add(
            name.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
            None,
        )
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
                let runner = cmd
                    .inner
                    .as_ref()
                    .as_any()
                    .and_then(|any| any.downcast_ref::<AliasRunner>());

                if runner.is_some() {
                    let prompt = format!("Remove alias '{}'", name);
                    if confirm(prompt, &scope, false).ok() == Some(Answer::Yes) {
                        let overriden = runner.unwrap().overriden.clone();
                        unregister_command(name);

                        if overriden.is_some() {
                            // Restore the previously overriden command
                            register_command(overriden.unwrap());
                        }
                    }
                    Ok(Value::success())
                } else {
                    Err(format_error(scope, name, args, "is not an alias"))
                }
            }
        }
    }

    #[cfg(test)]
    fn remove_all(&self, scope: &Arc<Scope>, args: &[String]) -> Result<Value, String> {
        for name in registered_commands(true) {
            _ = self.remove(&name, scope, args);
        }

        Ok(Value::success())
    }
}

impl Exec for Alias {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut parsed_args = flags.parse_relaxed(scope, args);

        if flags.is_present("help") {
            println!("Usage: {} [NAME EXPRESSION] [OPTIONS]", name);
            println!("Register or deregister aliases (expression shortcuts).");
            println!("\nOptions:");
            println!("{}", flags.help());
            println!();
            println!("Examples:");
            println!("    alias la \"ls -al\"");
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
        self.add(name, parsed_args, Some(scope.clone()))
    }
}

#[ctor::ctor]
fn register() {
    let alias = Alias::new();

    _ = alias.register("export", &["eval", "--export"]);
    _ = alias.register("source", &["eval", "--source"]);
    _ = alias.register("reset", &["clear", "--reset"]);

    #[cfg(windows)]
    {
        _ = alias.register("killall", &["taskkill", "/f", "/im"]);
        _ = alias.register("reboot", &["shutdown", "/r", "/t", "0"]);
    }

    register_command(ShellCommand {
        name: "alias".to_string(),
        inner: Arc::new(alias),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn setup() -> (Arc<Scope>, Alias) {
        let scope = Scope::new();
        let alias = Alias::new();

        scope.insert("NO_COLOR".to_string(), Value::Int(1));
        scope.insert("NO_CONFIRM".to_string(), Value::Int(1));

        alias.remove_all(&scope, &vec![]).unwrap();

        (scope, alias)
    }

    #[test]
    fn test_add_alias() {
        let (_scope, alias) = setup();
        let name = "la";
        let args = vec!["ls", "-al"];

        let result = alias.register(name, &args);
        assert!(result.is_ok());
        assert!(get_command(&name).is_some());
    }

    #[test]
    fn test_remove_alias() {
        let (scope, alias) = setup();
        let name = "la";
        let args = vec!["ls", "-al"];

        let result = alias.register(name, &args);
        assert!(result.is_ok());

        let result = alias.remove(&name, &scope, &vec![]);

        assert!(result.is_ok());
        assert!(get_command(&name).is_none());
    }

    #[test]
    fn test_remove_non_existent_alias() {
        let (scope, alias) = setup();
        let name = "non_existent".to_string();

        let result = alias.remove(&name, &scope, &vec![]);
        assert!(result.is_err());
        assert_eq!(result.err().unwrap(), name + ": alias not found");
    }

    #[test]
    fn test_remove_with_confirmation() {
        let (scope, alias) = setup();
        let name = "ll";
        let args = vec!["ls", "-al"];

        alias.register(name, &args).unwrap();

        let result = alias.remove(&name, &scope, &vec![]);
        assert!(result.is_ok());
        assert!(get_command(&name).is_none());
    }

    #[test]
    fn test_exec_with_list_flag() {
        let (scope, alias) = setup();
        let name = "la";
        let args = vec!["ls", "-al"];

        alias.register(name, &args).unwrap();

        let result = alias.exec("alias", &vec!["--list".to_string()], &scope);
        assert!(result.is_ok());
    }
}
