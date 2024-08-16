use super::{get_command, list_registered_commands, register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::rc::Rc;
use terminal_size::terminal_size;

struct Help {
    flags: CommandFlags,
}

impl Help {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        Help { flags }
    }

    fn print_interpreter_help() {
        println!("\nThis is a lightweight Unix-like command line interpreter.");
        println!("It supports various built-in commands and can execute external programs.");
        println!("\nUsage:");
        println!("  command [arguments]");
        println!("\nFor information on a specific command, type 'help <command>'.");
    }

    fn print_command_help(command: &str, scope: &Rc<Scope>) -> Result<(), String> {
        if command == "exit" {
            println!("exit [<exit code>]\n");
            Ok(())
        } else if command == "echo" {
            println!("echo [argument]...\n");
            Ok(())
        } else {
            match get_command(command) {
                Some(cmd) => {
                    let help_args = vec!["-?".to_string()];
                    cmd.exec(command, &help_args, scope)?;
                    Ok(())
                }
                None => Err(format!("Unknown command: '{}'", command)),
            }
        }
    }

    fn print_available_commands(spacing: usize) {
        println!("\nAvailable commands:");
        let commands = list_registered_commands(true);

        let max_width = terminal_size().map_or(80, |s| s.0 .0 as usize);
        let max_command_length = commands.iter().map(|cmd| cmd.len()).max().unwrap_or(0);
        let column_width = max_command_length + spacing;
        let num_columns = max_width / column_width;

        let mut current_column = 0;
        for cmd in commands {
            print!("{:<width$}", cmd, width = column_width);
            current_column += 1;
            if current_column >= num_columns {
                println!();
                current_column = 0;
            }
        }
        if current_column != 0 {
            println!();
        }
        println!();
    }
}

impl Exec for Help {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: help [COMMAND]");
            println!("Display information about the interpreter or specific commands.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            Self::print_interpreter_help();
            Self::print_available_commands(4);
        } else {
            for command in args {
                println!("\n");
                Self::print_command_help(&command, scope)?;
            }
        }

        Ok(Value::success())
    }

    fn is_external(&self) -> bool {
        false
    }
}

#[ctor::ctor]
fn register() {
    let help = Rc::new(Help::new());

    register_command(ShellCommand {
        name: "help".to_string(),
        inner: Rc::clone(&help) as Rc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "man".to_string(),
        inner: Rc::clone(&help) as Rc<dyn Exec>,
    });
}
