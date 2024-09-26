use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Ident, scope::Scope, scope::Variable};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;

struct Vars {
    flags: CommandFlags,
}

impl Vars {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('l', "local", "Display local scope variables only");
        flags.add_flag(
            'q',
            "quote",
            "Escape variable values and surround with double quotes",
        );

        Self { flags }
    }

    fn collect_vars(scope: &Arc<Scope>, local_only: bool) -> BTreeMap<Ident, Variable> {
        let mut all_vars = BTreeMap::new();
        let mut current_scope = Some(Arc::clone(scope));

        while let Some(scope) = current_scope {
            for (key, value) in scope.vars().iter() {
                if !all_vars.contains_key(key) {
                    all_vars.insert(key.clone(), value.clone());
                }
            }
            if local_only {
                break;
            }
            current_scope = scope.parent.as_ref().map(Arc::clone);
        }

        all_vars
    }
}

trait Escape {
    fn escape(&self, quote: bool) -> Cow<str>;
}

impl Escape for str {
    fn escape(&self, quote: bool) -> Cow<str> {
        if quote {
            Cow::Owned(format!("\"{}\"", self.escape_default()))
        } else {
            Cow::Borrowed(self)
        }
    }
}

impl Exec for Vars {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: vars [OPTIONS]");
            println!("Display variables visible in the current scope.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let quote = flags.is_present("quote");
        let local_only = flags.is_present("local");

        if !local_only && name == "env" {
            // Print the environment directly.
            let vars: Vec<String> = env::vars().map(|(key, _)| key).collect();

            for key in vars {
                my_println!(
                    "{}={}",
                    key,
                    env::var(&key).map_err(|e| e.to_string())?.escape(quote)
                )?;
            }
        } else {
            let vars = Self::collect_vars(scope, local_only);
            for (key, var) in vars {
                my_println!("{}={}", key, var.value().as_str().escape(quote))?;
            }
        }
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    let vars = Arc::new(Vars::new());

    register_command(ShellCommand {
        name: "env".to_string(),
        inner: Arc::clone(&vars) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "vars".to_string(),
        inner: Arc::clone(&vars) as Arc<dyn Exec>,
    });
}
