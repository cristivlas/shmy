use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::cmds::get_command;
use crate::eval::{Scope, Value};
use crate::prompt::read_password;
use crate::utils::interpreter_path;
use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::rc::Rc;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Threading::{
    CreateProcessWithLogonW, CREATE_UNICODE_ENVIRONMENT, LOGON_WITH_PROFILE, PROCESS_INFORMATION,
    STARTUPINFOW,
};

struct Sudo {
    flags: CommandFlags,
}

impl Sudo {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_option(
            'u',
            "user",
            "Specify the user to run as (default: Administrator)",
        );
        flags.add_option('-', "args", "Pass all remaining arguments to COMMAND");
        Sudo { flags }
    }
}

impl Exec for Sudo {
    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut command_args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: sudo [OPTIONS] COMMAND [ARGS]...");
            println!("Execute a command as another user (without passing environmental variables).");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if command_args.is_empty() {
            return Err("No command specified".to_string());
        }

        let user = flags
            .get_option("user")
            .unwrap_or("Administrator");
        let cmd_name = &command_args[0].to_string();

        if let Some(additional_args) = flags.get_option("args") {
            command_args.extend(additional_args.split_whitespace().map(String::from));
        }

        if let Some(cmd) = get_command(&cmd_name) {
            let password = read_password(&format!("Enter password for {}: ", user))
                .map_err(|e| format!("Failed to read password: {}", e))?;

            let command = if cmd.is_external() {
                command_args.join(" ")
            } else {
                let own_path =
                    interpreter_path().map_err(|e| format!("Failed to get own path: {}", e))?;
                format!("{} -c {}", own_path, command_args.join(" "))
            };

            let mut title: Vec<u16> = OsStr::new(&format!("{}: {}", user, cmd_name))
                .encode_wide()
                .chain(Some(0))
                .collect();

            let user: Vec<u16> = OsStr::new(&user).encode_wide().chain(Some(0)).collect();
            let password: Vec<u16> = OsStr::new(&password).encode_wide().chain(Some(0)).collect();

            let mut command: Vec<u16> = OsStr::new(&command).encode_wide().chain(Some(0)).collect();
            let mut startup_info = STARTUPINFOW::default();
            startup_info.lpTitle = PWSTR(title.as_mut_ptr());
            startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            let mut process_info = PROCESS_INFORMATION::default();

            // Construct environment block.

            // TODO: good idea (or useful at all?) to copy all vars to the environment here?
            // Not doing it for now.

            let env_vars = vec!["NO_COLOR=_"];
            let mut env_block: Vec<u16> = env_vars
                .iter()
                .flat_map(|s| OsStr::new(s).encode_wide().chain(Some(0)))
                .collect();
            // A Unicode environment block is terminated by four zero bytes:
            // two for the last string and two more to terminate the block.
            env_block.extend(vec![0, 0, 0, 0]);

            unsafe {
                CreateProcessWithLogonW(
                    PCWSTR(user.as_ptr()),
                    None,
                    PCWSTR(password.as_ptr()),
                    LOGON_WITH_PROFILE,
                    None,
                    PWSTR(command.as_mut_ptr()),
                    CREATE_UNICODE_ENVIRONMENT,
                    Some(env_block.as_mut_ptr() as *const _ as *const c_void),
                    None,
                    &mut startup_info,
                    &mut process_info,
                )
            }
            .map_err(|e| format!("Failed to create process with logon: {}", e))?;
            Ok(Value::success())
        } else {
            Err(format!("Command not found: {}", cmd_name))
        }
    }

    fn is_external(&self) -> bool {
        false
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "sudo".to_string(),
        inner: Rc::new(Sudo::new()),
    });
}
