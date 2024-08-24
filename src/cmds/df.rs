use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::utils::{format_size, root_path};
use crate::{eval::Value, scope::Scope};
use std::ffi::OsStr;
use std::io::Error;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives};

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

    fn disk_free_info(scope: &Rc<Scope>, path: &Path) -> Result<DiskFreeInfo, String> {
        let dirname: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();
        let mut info: DiskFreeInfo = DiskFreeInfo::new();

        let free_bytes_available_ptr = &mut info.free_bytes_available;
        let total_bytes_ptr = &mut info.total_bytes;
        let total_free_bytes_ptr = &mut info.total_free_bytes;

        unsafe {
            if GetDiskFreeSpaceExW(
                PCWSTR(dirname.as_ptr()),
                Some(free_bytes_available_ptr),
                Some(total_bytes_ptr),
                Some(total_free_bytes_ptr),
            )
            .is_err()
            {
                Err(format!(
                    "{}: {}",
                    scope.err_path(path),
                    Error::last_os_error()
                ))
            } else {
                Ok(info)
            }
        }
    }

    fn print_disk_free(
        scope: &Rc<Scope>,
        flags: &CommandFlags,
        path: &Path,
        max_len: usize,
    ) -> Result<(), String> {
        let info = Self::disk_free_info(scope, &path)?;

        let h = flags.is_present("human-readable");

        println!(
            "{:<max_len$} {:>16} {:>16} {:>16}",
            path.display(),
            format_size(info.free_bytes_available, 1, h),
            format_size(info.total_bytes, 1, h),
            format_size(info.total_free_bytes, 1, h)
        );
        Ok(())
    }
    fn print_disk_free_header(len: usize) {
        println!(
            "{:<len$} {:>16} {:>16} {:>16}",
            "Path", "Free", "Total", "Total Free"
        );
    }
}

fn path_from_str(scope: &Rc<Scope>, path: &str) -> Result<PathBuf, String> {
    let canonical_path = Path::new(path)
        .canonicalize()
        .map_err(|e| format!("{}: {}", scope.err_path_str(&path), e))?;

    Ok(root_path(&canonical_path))
}

fn enumerate_drives() -> Vec<String> {
    let mut roots = Vec::new();

    unsafe {
        let drives = GetLogicalDrives();

        for i in 0..26 {
            if (drives & (1 << i)) != 0 {
                let drive_letter = (b'A' + i as u8) as char;
                roots.push(format!("{}:\\", drive_letter));
            }
        }
    }

    roots
}

impl Exec for DiskFree {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut args = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: df [OPTIONS] [PATH]");
            println!("Display disk space usage for file systems.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            args = enumerate_drives();
        }

        let paths: Vec<PathBuf> = args
            .iter()
            .map(|path| path_from_str(scope, path))
            .collect::<Result<Vec<_>, _>>()?;

        // Compute the maximum path length across all processed paths
        let max_len = paths
            .iter()
            .map(|p| p.display().to_string().len())
            .max()
            .unwrap_or(40);

        Self::print_disk_free_header(max_len);

        for path in &paths {
            Self::print_disk_free(scope, &flags, &path, max_len)?;
        }
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
