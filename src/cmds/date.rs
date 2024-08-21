use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use chrono::{DateTime, Local, Utc};
use chrono_tz::Tz;
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
        flags.add_value_flag(
            'z',
            "timezone",
            "Specify the zone (e.g., America/New_York) to display local time",
        );
        Self { flags }
    }

    fn get_time_in_timezone(&self, zone: &String) -> Result<DateTime<Tz>, String> {
        let tz: Tz = zone
            .parse()
            .map_err(|_| format!("Invalid timezone specified: {}", zone))?;
        Ok(Utc::now().with_timezone(&tz))
    }

    fn format_time(&self, time: DateTime<Tz>, flags: &CommandFlags) -> String {
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
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: date [OPTIONS]");
            println!("Display the current date and time.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let use_utc = flags.is_present("utc");
        let timezone = if use_utc {
            "UTC".to_string()
        } else {
            flags.get_value("timezone").unwrap_or("UTC".to_string())
        };

        let current_time = self.get_time_in_timezone(&timezone)?;
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
