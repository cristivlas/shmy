use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::utils::{format_error, format_size, win::root_path};
use crate::{eval::Value, scope::Scope};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::io::Error;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_NO_MORE_FILES, MAX_PATH};
use windows::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetDiskFreeSpaceExW, GetLogicalDrives,
};

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

fn string_from_wide(wide: &mut Vec<u16>) -> String {
    let sz = wide.iter().position(|c| *c == 0).unwrap_or(wide.len());
    wide.resize(sz, 0);
    OsString::from_wide(wide).to_string_lossy().to_string()
}

impl DiskFree {
    fn new() -> Self {
        let mut flags = CommandFlags::with_help();
        flags.add_flag(
            'h',
            "human-readable",
            "Print sizes in human readable format (e.g., 1.1G)",
        );
        flags.add_flag('a', "all", "Enumerate All volumes");

        Self { flags }
    }

    fn disk_free_info(
        scope: &Arc<Scope>,
        path: &Path,
        args: &[String],
    ) -> Result<DiskFreeInfo, String> {
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
                    scope.err_path_arg(&path.display().to_string(), args),
                    Error::last_os_error()
                ))
            } else {
                Ok(info)
            }
        }
    }

    fn print_disk_free(
        scope: &Arc<Scope>,
        flags: &CommandFlags,
        path: &Path,
        max_len: usize,
        args: &[String],
    ) -> Result<(), String> {
        let info = Self::disk_free_info(scope, &path, args)?;

        let h = flags.is_present("human-readable");

        my_println!(
            "{:<max_len$} {:>16} {:>16}   {:3.2}%",
            path.display(),
            format_size(info.total_bytes, 1, h),
            format_size(info.total_free_bytes, 1, h),
            info.total_free_bytes as f64 * 100.0 / info.total_bytes as f64
        )?;
        Ok(())
    }

    fn print_disk_free_header(len: usize) -> Result<(), String> {
        my_println!(
            "{:<len$} {:>16} {:>16} {:>8}",
            "Path",
            "Total",
            "Free",
            "% Free"
        )
    }
}

fn root_path_from_str(scope: &Arc<Scope>, path: &str, args: &[String]) -> Result<PathBuf, String> {
    let canonical_path = Path::new(path)
        .canonicalize()
        .map_err(|e| format_error(scope, path, args, e))?;

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

fn enumerate_volumes() -> Vec<String> {
    let mut volumes = Vec::new();
    let mut volume_name: Vec<u16> = vec![0u16; MAX_PATH as usize + 1];

    unsafe {
        // Start volume enumeration
        let find_handle = match FindFirstVolumeW(&mut volume_name) {
            Ok(h) => h,
            Err(error) => {
                eprintln!("Failed to find the first volume: {}", error);
                return volumes;
            }
        };
        volumes.push(string_from_wide(&mut volume_name));

        loop {
            volume_name.resize(MAX_PATH as usize + 1, 0);

            if let Err(error) = FindNextVolumeW(find_handle, &mut volume_name) {
                if error.code() == ERROR_NO_MORE_FILES.to_hresult() {
                    break;
                } else {
                    eprintln!("Failed to find the next volume: {}", error);
                    break;
                }
            }
            volumes.push(string_from_wide(&mut volume_name));
        }
        _ = FindVolumeClose(find_handle);
        volumes
    }
}

impl Exec for DiskFree {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let volumes = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: df [OPTIONS] [PATH]");
            println!("Display disk space usage for file systems.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        let paths: BTreeSet<PathBuf> = {
            let vec_paths: Vec<PathBuf> = if volumes.is_empty() {
                if flags.is_present("all") {
                    // Collect paths directly into a Vec<PathBuf>
                    enumerate_volumes()
                        .iter()
                        .map(|s| PathBuf::from(s))
                        .collect()
                } else {
                    // Collect results and handle errors
                    enumerate_drives()
                        .iter()
                        .map(|s| root_path_from_str(scope, s, args))
                        .collect::<Result<Vec<PathBuf>, String>>()?
                }
            } else {
                // Collect results and handle errors
                volumes
                    .iter()
                    .map(|s| root_path_from_str(scope, s, args))
                    .collect::<Result<Vec<PathBuf>, String>>()?
            };

            // De-dupe, in case the user has specified multiple directories on the same volume
            vec_paths.into_iter().collect()
        };

        // Compute the maximum path length across all processed paths
        let max_len = paths
            .iter()
            .map(|p| p.display().to_string().len())
            .max()
            .unwrap_or(40);

        Self::print_disk_free_header(max_len)?;

        for path in &paths {
            Self::print_disk_free(scope, &flags, &path, max_len, args).unwrap_or(());
        }
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "df".to_string(),
        inner: Arc::new(DiskFree::new()),
    });
}
