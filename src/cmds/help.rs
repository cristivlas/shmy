use super::{
    flags::CommandFlags, get_command, register_command, registered_commands, Exec, ShellCommand,
};
use crate::{eval::Value, scope::Scope};
use gag::BufferRedirect;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;
use terminal_size::terminal_size;

struct Help {
    flags: CommandFlags,
}

impl Help {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }

    fn print_interpreter_help() {
        println!("NAME");
        println!("    mysh - Lightweight command line interpreter");
        println!();
        println!("SYNOPSIS");
        println!("    mysh [-c COMMAND [ARGS]]");
        println!("    mysh SCRIPT_FILE");
        println!();
        println!("DESCRIPTION");
        println!("    mysh is a simple, lightweight command line interpreter with a few Unix-like built-in commands.");
        println!("    It supports variable assignment and evaluation, conditional statements, loops, arithmetic and");
        println!(
            "    logical operations, command execution evaluation, output redirection, and pipes."
        );
        println!();
        println!("EXPRESSIONS");
        println!("    Variable Assignment and Evaluation");
        println!("        i = 5; echo $i");
        println!("    Conditional Statements");
        println!("        if ($i > 0) (echo \"i is positive\") else (echo \"i is not positive\")");
        println!("    Loops");
        println!("        while ($i > 0) (echo $i; $i = $i - 1)");
        println!("        for f in *.rs; (echo $f; ls -l $f)");
        println!("    Arithmetic Operations");
        println!("        Supported: +, -, /, *");
        println!("    Logical Operations");
        println!("        Supported: ||, &&");
        println!();
        println!("COMMAND EXECUTION");
        println!(
            "    A command evaluates to true if it succeeds (exit code 0) and false otherwise."
        );
        println!("    Errors from failed commands are stored in the special variable $__errors.");
        println!("    If a command fails and its status is not evaluated, execution stops.");
        println!();
        println!("REDIRECTS");
        println!("    Output to file: command => file");
        println!("    Append to file: command =>> file");
        println!();
        println!("PIPES");
        println!("    Pipe output between commands:");
        println!("        <expression> | <expression>");
        println!("    Example:");
        println!("        ls -al | (echo \"\\t\\tHEADER\"; cat; echo \"\\t\\tFOOTER\")");
        println!();
        println!("SPECIAL VARIABLES");
        println!("    Redirect stdout: $__stdout");
        println!("    Redirect stderr: $__stderr");
        println!("    Examples:");
        println!("        __stderr = null; ls");
        println!("        __stderr = log.txt; ls -al");
        println!("        __stderr = __stdout; ls -al /");
        println!("        __stdout = some/path/file.txt; __stderr = 1; ls -al");
        println!();
        Self::print_available_commands(4, 4);
        println!("SEE ALSO");
        println!("    help [COMMAND]");
        println!("    https://github.com/cristivlas/mysh");
        println!();
        println!("AUTHOR");
        println!("    Written by Cristian Vlasceanu");
        println!();
    }

    fn print_command_help(command: &str, scope: &Arc<Scope>) -> Result<(), String> {
        match command {
            "exit" => {
                println!("NAME");
                println!("    exit - Exit the command line interpreter");
                println!();
                println!("SYNOPSIS");
                println!("    exit [<exit code>]");
                println!();
                println!("DESCRIPTION");
                println!("    Exit the command line interpreter with an optional exit code.");
            }
            "echo" => {
                println!("NAME");
                println!("    echo - Print arguments to the console");
                println!();
                println!("SYNOPSIS");
                println!("    echo [argument]...");
                println!();
                println!("DESCRIPTION");
                println!("    Print the given arguments to the console.");
            }
            _ => match get_command(command) {
                Some(cmd) => {
                    let mut redirect = BufferRedirect::stdout()
                        .map_err(|e| format!("Failed to set up buffer redirect: {}", e))?;

                    cmd.exec(command, &vec!["-?".to_string()], scope)?;
                    let mut output = String::new();
                    redirect
                        .read_to_string(&mut output)
                        .map_err(|e| format!("Failed to read output: {}", e))?;
                    drop(redirect);

                    Self::print_help_output(command, &output);
                }
                None => return Err(format!("Unknown command: {}", command)),
            },
        }
        Ok(())
    }

    fn print_help_output(name: &str, output: &str) {
        let mut lines: Vec<&str> = output.lines().collect();

        // Print the name and synopsis
        println!("NAME");
        println!("    {}", name);
        println!();
        println!("SYNOPSIS");
        println!("    {}", lines.remove(0));
        println!();

        // Print the description
        println!("DESCRIPTION");
        for line in lines {
            if line.trim() == "Options:" {
                println!("OPTIONS");
            } else {
                println!("    {}", line);
            }
        }
    }

    fn print_available_commands(indent: usize, spacing: usize) {
        let commands = registered_commands(true);
        if !commands.is_empty() {
            println!("BUILT-IN COMMANDS");
        }
        let max_width = terminal_size()
            .map_or(80, |s| s.0 .0 as usize)
            .saturating_sub(indent);
        let max_command_length = commands.iter().map(|cmd| cmd.len()).max().unwrap_or(0);
        let column_width = max_command_length + spacing;
        let num_columns = max_width / column_width;

        let mut current_column = 0;
        for cmd in commands {
            if current_column == 0 {
                print!("{}", " ".repeat(indent));
            }
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
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: help [COMMAND]");
            println!("Display information about the interpreter or specific commands.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            Self::print_interpreter_help();
        } else {
            for command in &args {
                Self::print_command_help(&command, scope)?;
                println!();
            }
        }

        Ok(Value::success())
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
