use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::eval::Value;
use crate::scope::Scope;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

struct Link {
    flags: CommandFlags,
}

struct Options {
    symbolic: bool,
    force: bool,
    target: Option<String>,
    link_name: Option<String>,
}

impl Link {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('s', "symbolic", "Make symbolic links instead of hard links");
        flags.add_flag('f', "force", "Remove existing destination files");
        flags.add_flag('?', "help", "Display this help and exit");

        Self { flags }
    }

    fn parse_args(&self, scope: &Arc<Scope>, args: &[String]) -> Result<Options, String> {
        let mut flags = self.flags.clone();
        let parsed_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            return Ok(Options {
                symbolic: false,
                force: false,
                target: None,
                link_name: None,
            });
        }

        if parsed_args.len() != 2 {
            return Err("Missing operand".to_string());
        }

        Ok(Options {
            symbolic: flags.is_present("symbolic"),
            force: flags.is_present("force"),
            target: Some(parsed_args[0].clone()),
            link_name: Some(parsed_args[1].clone()),
        })
    }

    fn print_help(&self) {
        println!("Usage: ln [OPTION]... TARGET LINK_NAME");
        println!("Create a link to TARGET with the name LINK_NAME.");
        println!("\nOptions:");
        print!("{}", self.flags.help());
    }
}

impl Exec for Link {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let opts = self.parse_args(scope, args)?;

        if opts.target.is_none() || opts.link_name.is_none() {
            self.print_help();
            return Ok(Value::success());
        }

        create_link(
            opts.target.as_ref().unwrap(),
            opts.link_name.as_ref().unwrap(),
            &opts,
            scope,
        )
    }
}

fn create_link(
    target: &str,
    link_name: &str,
    opts: &Options,
    scope: &Arc<Scope>,
) -> Result<Value, String> {
    let target_path = Path::new(target);
    let link_path = Path::new(link_name);

    if opts.force && link_path.exists() {
        fs::remove_file(link_path).map_err(|error| {
            format!(
                "Failed to remove existing {}: {}",
                scope.err_path(link_path),
                error
            )
        })?;
    }

    #[cfg(windows)]
    let result = if opts.symbolic {
        use std::os::windows::fs as windows_fs;
        if target_path.is_dir() {
            windows_fs::symlink_dir(target_path, link_path)
        } else {
            windows_fs::symlink_file(target_path, link_path)
        }
    } else {
        fs::hard_link(target_path, link_path)
    };

    #[cfg(unix)]
    let result = if opts.symbolic {
        use std::os::unix::fs as unix_fs;
        unix_fs::symlink(target_path, link_path)
    } else {
        fs::hard_link(target_path, link_path)
    };

    result.map_err(|e| format!("Failed to create link: {}", e))?;

    Ok(Value::success())
}

#[ctor::ctor]
fn register() {
    let exec = Rc::new(Link::new());

    register_command(ShellCommand {
        name: "ln".to_string(),
        inner: Rc::clone(&exec) as Rc<dyn Exec>,
    });
}
