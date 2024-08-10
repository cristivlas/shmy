use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::process;
use std::rc::Rc;

struct Exit {
    flags: CommandFlags,
}

impl Exit {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        Exit { flags }
    }
}

impl Exec for Exit {
    fn exec(&self, _name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: exit [exit_code]");
            println!("Exit the shell with the specified exit code (default: 0).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let exit_code = if args.len() > 1 {
            args[1]
                .parse::<i32>()
                .map_err(|_| "Invalid exit code. Please provide a valid integer.".to_string())?
        } else {
            0
        };

        process::exit(exit_code);
    }

    fn is_external(&self) -> bool {
        false
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "exit".to_string(),
        inner: Rc::new(Exit::new()),
    });
}
