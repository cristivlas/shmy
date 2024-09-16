use super::{flags::CommandFlags, get_command, register_command, Exec, ShellCommand};
use crate::utils::executable;
use crate::{eval::Value, scope::Scope};
use std::ffi::OsStr;
use std::io::{Error, IsTerminal};
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND};
use windows::Win32::System::Registry::HKEY;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHELLEXECUTEINFOW_0,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

struct Sudo {
    flags: CommandFlags,
}

impl Sudo {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_option('-', "args", "Pass all remaining arguments to COMMAND");
        Self { flags }
    }

    fn runas(&self, exe: &str, args: &str) -> Result<Value, String> {
        let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
        let file: Vec<u16> = OsStr::new(&exe).encode_wide().chain(Some(0)).collect();
        let params: Vec<u16> = OsStr::new(&args).encode_wide().chain(Some(0)).collect();

        let mut sei = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            lpDirectory: PCWSTR::null(),
            nShow: SW_SHOWNORMAL.0,
            hInstApp: HINSTANCE::default(),
            lpIDList: std::ptr::null_mut(),
            lpClass: PCWSTR::null(),
            hkeyClass: HKEY::default(),
            dwHotKey: 0,
            Anonymous: SHELLEXECUTEINFOW_0::default(),
            hProcess: HANDLE::default(),
        };

        unsafe {
            if ShellExecuteExW(&mut sei).is_err() {
                return Err(format!("ShellExecuteExW: {}", Error::last_os_error()));
            } else if sei.hProcess.is_invalid() {
                return Err(format!("{} {}: {}", exe, args, Error::last_os_error()));
            } else {
                WaitForSingleObject(sei.hProcess, INFINITE);
                let mut exit_code = 0;
                let result = GetExitCodeProcess(sei.hProcess, &mut exit_code);

                CloseHandle(sei.hProcess).map_err(|e| e.to_string())?;

                result.map_err(|e| e.to_string())?;

                if exit_code != 0 {
                    return Err(format!("exit code: {:X}", exit_code));
                }
            }
        }
        Ok(Value::success())
    }
}

impl Exec for Sudo {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut command_args = flags.parse_all(scope, args);

        if flags.is_present("help") {
            println!("Usage: sudo [OPTIONS] COMMAND [ARGS]...");
            println!("Execute a command with elevated privileges");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if command_args.is_empty() {
            return Err("No command specified".to_string());
        }

        if !std::io::stdin().is_terminal() {
            return Err("Cannot pipe or redirect input to elevated command".to_string());
        }

        if !std::io::stdout().is_terminal() || !std::io::stderr().is_terminal() {
            return Err("Cannot pipe or redirect output from elevated command".to_string());
        }

        let cmd_name = command_args.remove(0);

        if let Some(additional_args) = flags.option("args") {
            command_args.extend(additional_args.split_whitespace().map(String::from));
        }

        let (executable, parameters) = if let Some(cmd) = get_command(&cmd_name) {
            let cur_dir =
                std::env::current_dir().map_err(|e| format!("Could not get current dir: {}", e))?;

            if cmd.is_external() {
                let path = cmd.path().to_string_lossy().to_string();

                if cmd.is_script() && !path.ends_with(".msc") {
                    (
                        "cmd.exe".to_owned(),
                        format!(
                            "/K cd {} && {} {}",
                            cur_dir.display(),
                            cmd_name,
                            command_args.join(" ")
                        ),
                    )
                } else {
                    (path, command_args.join(" "))
                }
            } else {
                // Internal command, run it by spawning an instance of this shell.
                (
                    executable().map_err(|e| format!("Could not get executable path: {}", e))?,
                    format!("-k {} {}", cmd_name, command_args.join(" ")),
                )
            }
        } else {
            return Err(format!("Command not found: {}", cmd_name));
        };

        self.runas(&executable, &parameters)
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "sudo".to_string(),
        inner: Arc::new(Sudo::new()),
    });
}
