use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope, utils::format_error};
use chrono::prelude::*;
use chrono::{DateTime, Local, Utc};
use chrono_tz::Tz;
use std::sync::Arc;

struct Date {
    flags: CommandFlags,
}

impl Date {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('u', "utc", "Display time in UTC instead of local time");
        flags.add_flag('r', "rfc2822", "Display date and time in RFC 2822 format");
        flags.add_flag('I', "iso8601", "Display date in ISO 8601 format");
        flags.add_value(
            'z',
            "timezone",
            "Specify the zone (e.g., America/New_York) to display local time",
        );

        Self { flags }
    }

    fn get_time_in_timezone(
        &self,
        scope: &Arc<Scope>,
        args: &[String],
        zone: &str,
    ) -> Result<DateTime<Tz>, String> {
        let tz: Tz = zone
            .parse()
            .map_err(|error| format_error(scope, zone, args, error))?;

        Ok(Utc::now().with_timezone(&tz))
    }

    fn format_time<Tz: TimeZone>(&self, time: DateTime<Tz>, flags: &CommandFlags) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
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
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let _args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: date [OPTIONS]");
            println!("Display the current date and time.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let formatted_time = if flags.is_present("utc") {
            let utc_time = Utc::now();
            self.format_time(utc_time, &flags)
        } else if let Some(tz) = flags.value("timezone") {
            let tz_time = self.get_time_in_timezone(scope, args, tz)?;
            self.format_time(tz_time, &flags)
        } else {
            let local_time = Local::now();
            self.format_time(local_time, &flags)
        };

        println!("{}", formatted_time);
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "date".to_string(),
        inner: Arc::new(Date::new()),
    });
}
