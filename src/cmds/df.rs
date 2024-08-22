use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::utils::{format_size, root_path, win_get_last_err_msg};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

struct DiskFree {
    flags: CommandFlags,
}

#[derive(Debug)]
struct DiskFreeInfo {
    free_bytes_available: u64,
    total_bytes: u64,
    total_free_bytes: u64,
}

impl DiskFreeInfo {
    fn new() -> Self {
        Self {
            free_bytes_available: 0,
            total_bytes: 0,
            total_free_bytes: 0,
        }
    }
}

impl DiskFree {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag(
            'h',
            "human-readable",
            "Print sizes in human readable format (e.g., 1.1G)",
        );
        Self { flags }
    }

    fn get_disk_free_info(
        &self,
        scope: &Rc<Scope>,
        path: &PathBuf,
    ) -> Result<DiskFreeInfo, String> {
        let dirname: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();
        let mut info: DiskFreeInfo = DiskFreeInfo::new();

        let free_bytes_available_ptr = &mut info.free_bytes_available;
        let total_bytes_ptr = &mut info.total_bytes;
        let total_free_bytes_ptr = &mut info.total_free_bytes;

        unsafe {
            if GetDiskFreeSpaceExW(
                dirname.as_ptr(),
                free_bytes_available_ptr,
                total_bytes_ptr,
                total_free_bytes_ptr,
            ) == 0
            {
                return Err(format!(
                    "{}: {}",
                    scope.err_path(path),
                    win_get_last_err_msg()
                ));
            }
        }
        Ok(info)
    }
}

fn get_path_from_arg(scope: &Rc<Scope>, args: &Vec<String>) -> Result<PathBuf, String> {
    let path = if args.is_empty() {
        let root = PathBuf::from("/");
        root.canonicalize().unwrap_or(root)
    } else {
        let canonical_path = Path::new(&args[0])
            .canonicalize()
            .map_err(|e| format!("{}: {}", scope.err_path_str(&args[0]), e))?;

        root_path(&canonical_path)
    };

    Ok(path)
}

impl Exec for DiskFree {
    fn is_external(&self) -> bool {
        false
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: df [OPTIONS] [PATH]");
            println!("Display disk space usage for file systems.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let path = get_path_from_arg(scope, &args)?;
        let info = self.get_disk_free_info(scope, &path)?;

        let h = flags.is_present("human-readable");
        let len = path.display().to_string().len();

        println!(
            "{:>len$} {:>16} {:>16} {:>16}",
            path.display(),
            "Free",
            "Total",
            "Total Free"
        );
        println!(
            "{:>len$} {:>16} {:>16} {:>16}",
            "",
            format_size(info.free_bytes_available, 1, h),
            format_size(info.total_bytes, 1, h),
            format_size(info.total_free_bytes, 1, h)
        );

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "df".to_string(),
        inner: Rc::new(DiskFree::new()),
    });
}
