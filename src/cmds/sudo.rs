use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::cmds::get_command;
use crate::eval::{Scope, Value};
use crate::utils::{executable, win_get_last_err_msg};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::rc::Rc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HINSTANCE, HWND};
use windows::Win32::System::Registry::HKEY;
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHELLEXECUTEINFOW_0,
};
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

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
}

impl Exec for Sudo {
    fn exec(&self, _name: &str, args: &Vec<String>, _scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut command_args = flags.parse(args)?;

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

        let cmd_name = command_args.remove(0);

        if let Some(additional_args) = flags.get_option("args") {
            command_args.extend(additional_args.split_whitespace().map(String::from));
        }

        let mut n_show = SW_SHOWNORMAL.0;

        let (executable, parameters) = if let Some(cmd) = get_command(&cmd_name) {
            if cmd.is_external() {
                if cmd.is_script() {
                    n_show = SW_HIDE.0;
                    (
                        "cmd.exe".to_string(),
                        format!("/C {} {}", cmd_name, command_args.join(" ")),
                    )
                } else {
                    (cmd_name, command_args.join(" "))
                }
            } else {
                let interp = executable().map_err(|e| format!("Failed to get own path: {}", e))?;
                (
                    interp,
                    format!("-c {} {}", cmd_name, command_args.join(" ")),
                )
            }
        } else {
            return Err(format!("Command not found: {}", cmd_name));
        };

        let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
        let file: Vec<u16> = OsStr::new(&executable)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let params: Vec<u16> = OsStr::new(&parameters)
            .encode_wide()
            .chain(Some(0))
            .collect();

        let mut sei = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            hwnd: HWND::default(),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            lpDirectory: PCWSTR::null(),
            nShow: n_show,
            hInstApp: HINSTANCE::default(),
            lpIDList: std::ptr::null_mut(),
            lpClass: PCWSTR::null(),
            hkeyClass: HKEY::default(),
            dwHotKey: 0,
            Anonymous: SHELLEXECUTEINFOW_0::default(),
            hProcess: HANDLE::default(),
        };

        unsafe {
            if ShellExecuteExW(&mut sei).is_ok() {
                if !sei.hProcess.is_invalid() {
                    windows::Win32::System::Threading::WaitForSingleObject(
                        sei.hProcess,
                        windows::Win32::System::Threading::INFINITE,
                    );
                    let mut exit_code = 0;
                    let result = windows::Win32::System::Threading::GetExitCodeProcess(
                        sei.hProcess,
                        &mut exit_code,
                    );

                    CloseHandle(sei.hProcess).map_err(|e| e.to_string())?;

                    result.map_err(|e| e.to_string())?;
                    if exit_code != 0 {
                        return Err(format!("exit code: {:X}", exit_code));
                    }
                } else {
                    return Err(win_get_last_err_msg());
                }
            }

            Ok(Value::success())
        }
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "sudo".to_string(),
        inner: Rc::new(Sudo::new()),
    });
}
