use super::{
    flags::CommandFlags, get_command, register_command, registered_commands, Exec, Flag,
    ShellCommand,
};
use crate::{
    eval::Value,
    scope::Scope,
    utils::{self, executable},
};
use std::process::Command;
use std::sync::Arc;

struct Help {
    flags: CommandFlags,
}

impl Help {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");

        Self { flags }
    }

    #[rustfmt::skip]
    fn print_interpreter_help() {
        println!("NAME");
        println!("    shmy - Lightweight command line interpreter");
        println!();
        println!("SYNOPSIS");
        println!("    shmy [-c COMMAND [ARGS]]");
        println!("    shmy SCRIPT_FILE");
        println!();
        println!("DESCRIPTION");
        println!("    shmy is a simple, lightweight command line interpreter with a few Unix-like built-in commands.");
        println!("    It supports variable assignment and evaluation; conditional statements; loops; arithmetic and");
        println!("    logical operations; command execution evaluation; output redirection and pipes.");
        println!();
        println!("EXPRESSIONS");
        println!("    Variable Assignment and Evaluation");
        println!("        Example: i = 5; echo $i");
        println!("    Conditional Statements");
        println!("        Example: if ($i > 0) (echo \"i is positive\") else (echo \"i is not positive\")");
        println!("    Loops");
        println!("        Example: while ($i > 0) (echo $i; $i = $i - 1)");
        println!("        Example: for f in *.rs; (echo $f; ls -l $f)");
        println!();
        println!("    Arithmetic Operators");
        println!("        '+': add, '-': subtract, '/': divide, '//': divide integers, '%': modulo, '*': multiply, '^': exponent");
        println!("    Logical Operators");
        println!("        '||': or, '&&': and");
        println!();
        println!("COMMAND EXECUTION");
        println!("    A command evaluates to true if it succeeds (exit code 0) and false otherwise.");
        println!("    Errors from failed commands are stored in the special variable $__errors.");
        println!("    If a command fails and its status is not evaluated, execution stops.");
        println!();
        println!("REDIRECTS");
        println!("    Output to file: <command> => <file path>");
        println!("    Append to file: <command> =>> <file path>");
        println!();
        println!("PIPES");
        println!("    Pipe output between commands:");
        println!("        <expression> | <expression>");
        println!("    Example:");
        println!("        ls -al | (echo \"\\t\\tHEADER\"; cat; echo \"\\t\\tFOOTER\")");
        println!();
        println!("COMMAND HISTORY");
        println!("    The command history is saved in ~/.shmy/history.txt");
        println!("    The maximum number of entries saved in the history is controlled by $HISTFILESIZE");
        println!("    $HISTFILESIZE needs to be set in the environment before the shell starts.");
        println!("    Modifying $HISTFILESIZE from a shell session has no effect (this behavior may change).");
        println!();
        println!("PROMPT CUSTOMIZATION");
        println!("    The prompt can be customized using escape sequences prefixed with '\\'.");
        println!("    Supported sequences:");
        println!("        \\u  - Insert the current username");
        println!("        \\H  - Insert the full hostname");
        println!("        \\h  - Insert the short hostname (up to the first dot)");
        println!("        \\w  - Insert the current working directory");
        println!("        \\$  - Insert '#' if the user is root, otherwise '$'");
        println!();
        println!("    Examples:");
        println!("        $__prompt = \\u@\\h:\\w\\_");
        println!("        $__prompt = \"\\\\u@\\h|\\\\w\\\\$ \"");
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
        println!("    https://github.com/cristivlas/shmy");
        println!();
        println!("AUTHOR");
        println!("    Written by Cristian Vlasceanu");
        println!();
    }

    fn print_command_help(command: &str, _scope: &Arc<Scope>) -> Result<(), String> {
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
                    let mut std_cmd = Command::new(executable()?);
                    let child = std_cmd
                        .arg("-c")
                        .arg(cmd.name())
                        .arg("-?")
                        .stdout(std::process::Stdio::piped())
                        .spawn()
                        .map_err(|e| e.to_string())?;

                    let output = child.wait_with_output().map_err(|e| e.to_string())?;
                    Self::print_help_output(command, &String::from_utf8_lossy(&output.stdout));
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
        let max_width = utils::terminal_width().saturating_sub(indent);
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

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
    let help = Arc::new(Help::new());

    register_command(ShellCommand {
        name: "help".to_string(),
        inner: Arc::clone(&help) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "man".to_string(),
        inner: Arc::clone(&help) as Arc<dyn Exec>,
    });
}
