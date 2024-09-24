use super::{flags::CommandFlags, register_command, Exec, Flag, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::sync::Arc;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

struct PowerStatus {
    flags: CommandFlags,
}

impl PowerStatus {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag('a', "ac", "Show AC line status");
        flags.add_flag('b', "battery", "Show battery percentage");

        Self { flags }
    }

    fn get_power_status() -> windows::core::Result<(String, String)> {
        let mut status = SYSTEM_POWER_STATUS::default();
        unsafe {
            GetSystemPowerStatus(&mut status)?;
        }
        let ac_status = match status.ACLineStatus {
            0 => "Offline".to_string(),
            1 => "Online".to_string(),
            _ => "Unknown".to_string(),
        };
        let battery_status = format!("{}%", status.BatteryLifePercent);
        Ok((ac_status, battery_status))
    }

    fn print_result(
        ac_status: &str,
        battery_status: &str,
        flags: &CommandFlags,
    ) -> Result<(), String> {
        if flags.is_empty() || flags.is_present("ac") {
            println!("AC: {}", ac_status);
        }
        if flags.is_empty() || flags.is_present("battery") {
            println!("Battery: {}", battery_status);
        }
        Ok(())
    }
}

impl Exec for PowerStatus {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        _ = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: power [OPTION]...");
            println!("Display the power status, including AC and battery levels.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        match PowerStatus::get_power_status() {
            Ok((ac_status, battery_status)) => {
                PowerStatus::print_result(&ac_status, &battery_status, &flags)?;
            }
            Err(e) => return Err(format!("Failed to get power status: {}", e)),
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "power".to_string(),
        inner: Arc::new(PowerStatus::new()),
    });
}
