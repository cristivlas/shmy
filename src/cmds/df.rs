use super::{register_command, Exec, RegisteredCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use std::process::Command;
use std::rc::Rc;

struct DiskFree {
    flags: CommandFlags,
}

impl DiskFree {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message", false);
        DiskFree { flags }
    }

    fn print_usage(&self, path: &str) {
        let output = if cfg!(target_os = "windows") {
            Command::new("wmic")
                .args(&["logicaldisk", "get", "size,freespace,caption"])
                .output()
                .expect("Failed to execute command")
        } else {
            Command::new("df")
                .arg("-h")
                .arg(path)
                .output()
                .expect("Failed to execute command")
        };

        let result = String::from_utf8_lossy(&output.stdout);
        println!("{}", result);
    }
}

impl Exec for DiskFree {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: df [OPTIONS] [PATH]");
            println!("Display disk space usage for file systems.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let path = args.get(0).map_or("/", |s| s.as_str());
        self.print_usage(path);
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(RegisteredCommand {
        name: "df".to_string(),
        inner: Rc::new(DiskFree::new()),
    });
}
