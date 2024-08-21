use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use chrono::{DateTime, Local, Utc};
use std::rc::Rc;

struct Date {
    flags: CommandFlags,
}

impl Date {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('u', "utc", "Display time in UTC instead of local time");
        flags.add_flag('r', "rfc2822", "Display date and time in RFC 2822 format");
        flags.add_flag('I', "iso8601", "Display date in ISO 8601 format");
        Self { flags }
    }

    fn get_current_time(&self, use_utc: bool) -> DateTime<chrono::FixedOffset> {
        if use_utc {
            Utc::now().into()
        } else {
            Local::now().into()
        }
    }

    fn format_time(&self, time: DateTime<chrono::FixedOffset>, flags: &CommandFlags) -> String {
        if flags.is_present("rfc2822") {
            time.to_rfc2822()
        } else if flags.is_present("iso8601") {
            time.to_rfc3339()
        } else {
            time.format("%Y-%m-%d %H:%M:%S %z").to_string()
        }
    }
}

impl Exec for Date {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let _args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: date [OPTIONS]");
            println!("Display the current date and time.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let use_utc = flags.is_present("utc");
        let current_time = self.get_current_time(use_utc);
        let formatted_time = self.format_time(current_time, &flags);

        println!("{}", formatted_time);
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "date".to_string(),
        inner: Rc::new(Date::new()),
    });
}
