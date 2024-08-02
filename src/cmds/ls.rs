use super::{register_command, BuiltinCommand, Exec};
use crate::eval::{Scope, Value};
use chrono::DateTime;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
struct Dir;

const OWNER_MAX_LEN: usize = 14;

struct CmdArgs {
    all_files: bool,
    show_details: bool,
    megabytes: bool,
    paths: Vec<String>,
}

impl Exec for Dir {
    fn exec(&self, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        list_directories(&parse_args(&args))
    }
}

fn parse_args(args: &[String]) -> CmdArgs {
    let mut cmd_args = CmdArgs {
        all_files: false,
        show_details: false,
        megabytes: false,
        paths: Vec::new(),
    };

    for arg in args {
        if arg.starts_with('-') {
            for flag in arg.chars().skip(1) {
                match flag {
                    'a' => cmd_args.all_files = true,
                    'l' => cmd_args.show_details = true,
                    'h' => cmd_args.megabytes = true,
                    _ => {
                        eprintln!("Warning: unknown flag ignored: -{}", flag);
                    }
                }
            }
        } else {
            cmd_args.paths.push(arg.clone());
        }
    }
    if cmd_args.paths.is_empty() {
        cmd_args.paths.push(".".to_string());
    }
    cmd_args
}

fn format_file_type(metadata: &fs::Metadata) -> char {
    if metadata.is_dir() {
        'd'
    } else if metadata.is_file() {
        '-'
    } else if metadata.is_symlink() {
        'l'
    } else {
        '?'
    }
}

fn format_time(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    if let Some(datetime) = DateTime::from_timestamp(duration.as_secs() as i64, 0) {
        datetime.format("%b %d %H:%M").to_string()
    } else {
        "?".to_owned()
    }
}

#[cfg(unix)]
fn get_owner_and_group(_: PathBuf, metadata: &fs::Metadata) -> (String, String) {
    use std::os::unix::fs::MetadataExt;
    use users::{get_group_by_gid, get_user_by_uid};

    let uid = metadata.uid();
    let gid = metadata.gid();

    let owner = get_user_by_uid(uid)
        .map(|user| user.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| uid.to_string());

    let group = get_group_by_gid(gid)
        .map(|group| group.name().to_string_lossy().into_owned())
        .unwrap_or_else(|| gid.to_string());

    (owner, group)
}

#[cfg(not(any(unix, windows)))]
fn get_owner_and_group(_: PathBuf, _: &fs::Metadata) -> (String, String) {
    ("-".to_string(), "-".to_string())
}

#[cfg(windows)]
fn get_owner_and_group_sids(
    mut path: PathBuf,
    metadata: &fs::Metadata,
) -> (Option<String>, Option<String>) {
    use std::fs::OpenOptions;
    use std::os::windows::prelude::*;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{HANDLE, PSID, WIN32_ERROR};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetSecurityInfo, SE_OBJECT_TYPE,
    };
    use windows::Win32::Security::PSECURITY_DESCRIPTOR;
    use windows_sys::Win32::System::Memory::LocalFree;

    let get_sid_string = |psid: PSID| unsafe {
        let mut sid_string_ptr = PWSTR::null();
        if ConvertSidToStringSidW(psid, &mut sid_string_ptr).as_bool() {
            let sid_string = sid_string_ptr
                .to_string()
                .unwrap_or_else(|_| "-".to_string());
            LocalFree(sid_string_ptr.0 as isize);
            Some(sid_string)
        } else {
            None
        }
    };
    if metadata.is_symlink() {
        match fs::read_link(path) {
            Ok(p) => path = p,
            Err(_) => return (None, None),
        }
    }

    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return (None, None),
    };

    let handle = HANDLE(file.as_raw_handle() as isize);

    unsafe {
        let mut psid_owner: PSID = PSID::default();
        let mut psid_group: PSID = PSID::default();
        let mut sd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR::default();

        let result = GetSecurityInfo(
            handle,
            SE_OBJECT_TYPE(1i32), // SE_FILE_OBJECT
            3u32,                 // OWNER_SECURITY_INFORMATION + GROUP_SECURITY_INFORMATION
            Some(&mut psid_owner),
            Some(&mut psid_group),
            None,
            None,
            Some(&mut sd),
        );

        if result != WIN32_ERROR(0) {
            LocalFree(sd.0 as isize);
        } else {
            let owner = get_sid_string(psid_owner);
            let group = get_sid_string(psid_group);

            return (owner, group);
        }
        return (None, None);
    }
}

#[cfg(windows)]
fn name_from_sid(opt_sid: Option<String>) -> String {
    use std::cmp::min;
    use std::ffi::c_void;
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::LookupAccountSidW;
    use windows_sys::Win32::System::Memory::LocalFree;

    if let Some(sid) = opt_sid {
        unsafe {
            let mut psid: *mut c_void = std::ptr::null_mut();
            let wide_sid: Vec<u16> = sid.encode_utf16().chain(std::iter::once(0)).collect();

            if ConvertStringSidToSidW(wide_sid.as_ptr(), &mut psid) == 0 {
                return sid[..OWNER_MAX_LEN].to_string();
            }

            let mut name_size: u32 = 0;
            let mut domain_size: u32 = 0;
            let mut sid_use: i32 = 0;

            // First call to get buffer sizes
            LookupAccountSidW(
                std::ptr::null(),
                psid,
                std::ptr::null_mut(),
                &mut name_size,
                std::ptr::null_mut(),
                &mut domain_size,
                &mut sid_use,
            );

            let mut name = vec![0u16; name_size as usize];
            let mut domain = vec![0u16; domain_size as usize];

            // Second call to get actual data
            if LookupAccountSidW(
                std::ptr::null(),
                psid,
                name.as_mut_ptr(),
                &mut name_size,
                domain.as_mut_ptr(),
                &mut domain_size,
                &mut sid_use,
            ) != 0
            {
                LocalFree(psid as isize);
                name_size = min(name_size, OWNER_MAX_LEN as u32);
                String::from_utf16_lossy(&name[..name_size as usize])
            } else {
                LocalFree(psid as isize);
                sid[..OWNER_MAX_LEN].to_string()
            }
        }
    } else {
        "-".to_string()
    }
}

#[cfg(windows)]
fn get_owner_and_group(path: PathBuf, metadata: &fs::Metadata) -> (String, String) {
    let (owner_sid, group_sid) = get_owner_and_group_sids(path, metadata);
    (name_from_sid(owner_sid), name_from_sid(group_sid))
}

#[cfg(unix)]
fn get_permissions(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode();
    let flags = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];

    let mut perms = String::with_capacity(9);
    for &(bit, ch) in &flags {
        perms.push(if mode & bit != 0 { ch } else { '-' });
    }

    perms
}

#[cfg(windows)]
fn get_permissions(metadata: &fs::Metadata) -> String {
    use std::os::windows::fs::MetadataExt;

    let attrs = metadata.file_attributes();
    let mut perms = String::with_capacity(9);

    // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileattributesa
    perms.push(if attrs & 0x1 != 0 { 'r' } else { '-' }); // Read-only
    perms.push(if attrs & 0x2 != 0 { 'h' } else { '-' }); // Hidden
    perms.push(if attrs & 0x4 != 0 { 's' } else { '-' }); // System
    perms.push(if attrs & 0x20 != 0 { 'a' } else { '-' }); // Archive
    perms.push(if attrs & 0x80 != 0 { 'n' } else { '-' }); // Normal
    perms.push(if attrs & 0x100 != 0 { 't' } else { '-' }); // Temporary

    perms
}

#[cfg(not(any(unix, windows)))]
fn get_permissions(_metadata: &fs::Metadata) -> String {
    "---------".to_string()
}

fn list_directories(args: &CmdArgs) -> Result<Value, String> {
    for path in &args.paths {
        let entries = fs::read_dir(path).map_err(|e| format!("cannot access '{}': {}", path, e))?;

        let mut entries: Vec<_> = entries
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Error reading entries: {}", e))?;

        entries.sort_by_key(|e| e.file_name());

        if args.show_details {
            println!("total {}", entries.len());
            for entry in &entries {
                let metadata = entry
                    .metadata()
                    .map_err(|e| format!("Failed to get metadata: {}", e))?;
                let mut file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with(".") && !args.all_files {
                    continue;
                }
                let size = if args.megabytes {
                    format!("{:.2}M", metadata.len() as f64 / 1_048_576.0)
                } else {
                    metadata.len().to_string()
                };
                if metadata.is_symlink() {
                    if let Ok(path) = fs::read_link(entry.path()) {
                        file_name = format!("{} -> {}", file_name, path.to_string_lossy());
                    }
                }
                let file_type = format_file_type(&metadata);
                let modified_time = format_time(metadata.modified().unwrap_or(UNIX_EPOCH));
                let (owner, group) = get_owner_and_group(entry.path(), &metadata);
                let permissions = get_permissions(&metadata);
                println!(
                    "{}{}  {:OWNER_MAX_LEN$} {:OWNER_MAX_LEN$} {:>12}  {}  {}",
                    file_type, permissions, owner, group, size, modified_time, file_name
                );
            }
        } else {
            let max_width = entries
                .iter()
                .map(|e| e.file_name().to_string_lossy().len())
                .max()
                .unwrap_or(0);
            let column_width = max_width + 2;
            let terminal_width = 80; // Assume 80 columns if we can't detect
            let columns = terminal_width / column_width;

            for (index, entry) in entries.iter().enumerate() {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name.starts_with(".") && !args.all_files {
                    continue;
                }
                print!("{:<width$}", file_name, width = column_width);
                if (index + 1) % columns == 0 {
                    println!();
                }
            }
            if entries.len() % columns != 0 {
                println!();
            }
        }
    }

    Ok(Value::Int(0))
}

#[ctor::ctor]
fn register() {
    let exec = Rc::new(Dir);

    register_command(BuiltinCommand {
        name: "ls".to_string(),
        inner: Rc::clone(&exec) as Rc<dyn Exec>,
    });

    register_command(BuiltinCommand {
        name: "dir".to_string(),
        inner: Rc::clone(&exec) as Rc<dyn Exec>,
    });
}
