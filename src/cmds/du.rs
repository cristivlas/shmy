use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::utils::format_size;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct DiskUtilization {
    flags: CommandFlags,
}

impl Exec for DiskUtilization {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let mut paths: Vec<String> = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: du [OPTIONS] [PATH...]");
            println!("Estimate file space usage.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if paths.is_empty() {
            paths.push(".".to_string());
        }

        let opts = Options {
            all: flags.is_present("all"),
            apparent: flags.is_present("apparent"),
            summarize: flags.is_present("summarize"),
            human: flags.is_present("human-readable"),
            block_size: 1024,
            max_depth: flags
                .get_option("max-depth")
                .map(|s| s.parse().unwrap_or(usize::MAX)),
        };

        for p in &paths {
            let mut file_ids = HashSet::new();
            let path = PathBuf::from(p);
            let size = du_size(&path, &opts, scope, 0, &mut file_ids)?;

            if opts.summarize {
                print_size(&path, size, &opts)?;
            }
        }
        Ok(Value::success())
    }
}

impl DiskUtilization {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag(
            'a',
            "all",
            "Include counts for all files, not just directories",
        );
        flags.add_flag('s', "summarize", "Display only a total for each argument");
        flags.add_flag(
            'h',
            "human-readable",
            "Print sizes in human readable format (e.g., 1.1K, 234M, 2.7G)",
        );
        flags.add_option(
            'd',
            "max-depth",
            "Print the total for a directory only if below the specified depth",
        );
        flags.add(
            None,
            "apparent",
            false,
            "Print apparent sizes, rather than disk usage",
        );

        Self { flags }
    }
}

struct Options {
    all: bool,
    apparent: bool,
    human: bool,
    summarize: bool,
    block_size: u64,
    max_depth: Option<usize>,
}

fn du_size(
    path: &Path,
    opts: &Options,
    scope: &Rc<Scope>,
    depth: usize,
    file_ids: &mut HashSet<(u64, u64)>,
) -> Result<u64, String> {
    // Skip symbolic links
    if path.is_symlink() {
        return Ok(0);
    }

    let mut size: u64 = estimate_disk_size(scope, &opts, file_ids, path)?;

    if path.is_dir() {
        match fs::read_dir(path) {
            Err(e) => {
                my_warning!(scope, "{}: {}", scope.err_path(path), e);
            }
            Ok(dir) => {
                for entry in dir {
                    if scope.is_interrupted() {
                        return Ok(size);
                    }

                    let entry = entry.map_err(|e| format!("{}: {}", scope.err_path(path), e))?;
                    size += du_size(&entry.path(), &opts, scope, depth + 1, file_ids)?;
                }
            }
        }
    }

    if !opts.summarize && depth <= opts.max_depth.unwrap_or(usize::MAX) {
        if opts.all || path.is_dir() {
            print_size(path, size, opts)?;
        }
    }

    Ok(size)
}

fn estimate_disk_size(
    scope: &Rc<Scope>,
    opts: &Options,
    file_ids: &mut HashSet<(u64, u64)>,
    path: &Path,
) -> Result<u64, String> {
    #[cfg(unix)]
    {
        unix_disk_size(scope, opts, file_ids, path)
    }
    #[cfg(windows)]
    {
        win::disk_size(scope, opts, file_ids, path)
    }
}

#[cfg(unix)]
fn unix_disk_size(
    scope: &Rc<Scope>,
    opts: &Options,
    file_ids: &mut HashSet<(u64, u64)>,
    path: &Path,
) -> Result<u64, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path).map_err(|e| format!("{}: {}", scope.err_path(path), e))?;

    // Avoid double-counting hard links
    let inode = (metadata.dev(), metadata.ino());
    if file_ids.get(&inode).is_some() {
        return Ok(0);
    }
    file_ids.insert(inode);

    if opts.apparent {
        Ok(metadata.len())
    } else {
        // https://doc.rust-lang.org/std/os/unix/fs/trait.MetadataExt.html#tymethod.blocks
        // num_blocks is in 512 units, NOT metadata.blksize()
        let num_blocks = metadata.blocks();

        Ok(num_blocks * 512)
    }
}

#[cfg(windows)]
mod win {
    use super::Options;
    use crate::eval::{Scope, Value};
    use crate::utils::root_path;
    use std::collections::HashSet;
    use std::ffi::OsStr;
    use std::fs;
    use std::fs::OpenOptions;
    use std::io::Error;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::rc::Rc;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceW;
    use windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    pub fn disk_size(
        scope: &Rc<Scope>,
        opts: &Options,
        file_ids: &mut HashSet<(u64, u64)>,
        path: &Path,
    ) -> Result<u64, String> {
        let metadata =
            fs::metadata(path).map_err(|e| format!("{}: {}", scope.err_path(path), e))?;

        let id = unique_file_id(scope, path)?;

        // Check if we've seen this file before, avoid double-counting hard links
        if file_ids.contains(&id) {
            return Ok(0);
        }
        file_ids.insert(id);

        if opts.apparent {
            Ok(metadata.len())
        } else {
            let block_size = block_size(scope, &root_path(&path))?;
            Ok(((metadata.file_size() + block_size - 1) / block_size) * block_size)
        }
    }

    fn block_size(scope: &Rc<Scope>, root_path: &Path) -> Result<u64, String> {
        let cache_var = format!("blksz_{}", root_path.display());
        if let Some(v) = scope.lookup_value(&cache_var) {
            return u64::try_from(v);
        }

        let path_wide: Vec<u16> = OsStr::new(root_path)
            .encode_wide()
            .chain(Some(0).into_iter())
            .collect();

        let mut sectors_per_cluster = 0;
        let mut bytes_per_sector = 0;
        let mut _free_clusters = 0;
        let mut _total_clusters = 0;

        unsafe {
            if GetDiskFreeSpaceW(
                PCWSTR(path_wide.as_ptr()),
                Some(&mut sectors_per_cluster),
                Some(&mut bytes_per_sector),
                Some(&mut _free_clusters),
                Some(&mut _total_clusters),
            )
            .is_err()
            {
                return Err(format!(
                    "Failed to get disk space info for {}: {}",
                    scope.err_path(root_path),
                    Error::last_os_error()
                ));
            }
        }

        // Calculate block size
        let block_size = sectors_per_cluster as u64 * bytes_per_sector as u64;

        // Cache it in the current scope
        scope.insert(cache_var, Value::Int(block_size as _));

        Ok(block_size)
    }

    /// Build a unique id from the volume serial number and the file index.
    /// Used with a hash set to avoid double counting of hard link.
    fn unique_file_id(scope: &Rc<Scope>, path: &Path) -> Result<(u64, u64), String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(path)
            .map_err(|e| format!("Failed to open file {}: {}", path.display(), e))?;

        let handle = HANDLE(file.as_raw_handle());
        let mut file_info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };

        let result = unsafe { GetFileInformationByHandle(handle, &mut file_info) };

        if result.is_err() {
            return Err(format!(
                "Failed to get file information: {} {}",
                scope.err_path(path),
                Error::last_os_error()
            ));
        }

        let volume_serial_number = file_info.dwVolumeSerialNumber as u64;
        let file_index =
            ((file_info.nFileIndexHigh as u64) << 32) | (file_info.nFileIndexLow as u64);

        Ok((volume_serial_number, file_index))
    }
}

fn print_size(path: &Path, size: u64, opts: &Options) -> Result<(), String> {
    my_println!(
        "{}\t{}",
        format_size(size, opts.block_size, opts.human),
        path.display()
    )
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "du".to_string(),
        inner: Rc::new(DiskUtilization::new()),
    });
}
