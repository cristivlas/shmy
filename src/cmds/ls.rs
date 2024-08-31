use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::utils::{format_size, resolve_links};
use crate::{eval::Value, scope::Scope, wsl::IsWslLink};
use chrono::{DateTime, Local, Utc};
use colored::*;
use core::fmt;
use std::fs::{self, DirEntry, Metadata};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use terminal_size::{terminal_size, Width};

struct ColorScheme {
    use_colors: bool,
    scope: Rc<Scope>,
}

impl ColorScheme {
    fn with_scope(scope: &Rc<Scope>) -> Self {
        Self {
            use_colors: scope.use_colors(&std::io::stdout()),
            scope: Rc::clone(&scope),
        }
    }

    fn render_error<E: fmt::Display>(&self, e: &E) -> ColoredString {
        if self.use_colors {
            e.to_string().bright_red()
        } else {
            e.to_string().normal()
        }
    }

    fn render_error_path(&self, path: &Path) -> ColoredString {
        self.scope.err_path(path)
    }

    fn render_file_name(&self, file_name: &str, metadata: &Metadata) -> ColoredString {
        if self.use_colors {
            if metadata.is_dir() {
                return file_name.blue().bold();
            } else if metadata.is_symlink() {
                return file_name.cyan().bold();
            }
        }
        return file_name.normal();
    }

    fn render_file_type(&self, file_type: &str) -> ColoredString {
        if self.use_colors {
            file_type.blue()
        } else {
            file_type.normal()
        }
    }

    fn render_permissions(&self, perm: String) -> ColoredString {
        if self.use_colors {
            perm.cyan()
        } else {
            perm.normal()
        }
    }

    fn render_size(&self, path: &Path, mut size: String) -> ColoredString {
        let is_wsl = path.is_wsl_link().unwrap_or(false);
        if is_wsl {
            size = "WSL".to_string();
        }
        if self.use_colors {
            if is_wsl {
                size.bright_cyan()
            } else {
                size.green()
            }
        } else {
            size.normal()
        }
    }

    fn render_mod_time(&self, time: String) -> ColoredString {
        if self.use_colors {
            time.purple()
        } else {
            time.normal()
        }
    }
}

struct Dir {
    flags: CommandFlags,
}

const OWNER_MAX_LEN: usize = 16;

struct Options {
    all_files: bool,
    show_details: bool,
    human_readable: bool,
    help: bool,
    paths: Vec<String>,
    colors: ColorScheme,
    use_utc: bool,
}

impl Dir {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('a', "all", "Do not ignore entries starting with .");
        flags.add_flag('l', "long", "Use a long listing format");
        flags.add_flag(
            'h',
            "human-readable",
            "Print sizes in human readable format (e.g., 1K, 234M, 2G)",
        );
        flags.add_flag('u', "utc", "Show file times in UTC");
        flags.add_flag('?', "help", "Display this help and exit");

        Self { flags }
    }

    fn parse_args(&self, scope: &Rc<Scope>, args: &[String]) -> Result<Options, String> {
        let mut flags = self.flags.clone();
        let parsed_args = flags.parse(scope, args)?;

        let cmd_args = Options {
            all_files: flags.is_present("all"),
            show_details: flags.is_present("long"),
            human_readable: flags.is_present("human-readable"),
            help: flags.is_present("help"),
            paths: if parsed_args.is_empty() {
                vec![".".to_string()]
            } else {
                parsed_args
            },
            colors: ColorScheme::with_scope(&scope),
            use_utc: flags.is_present("utc"),
        };

        Ok(cmd_args)
    }

    fn print_help(&self) {
        println!("Usage: ls [OPTION]... [FILE]...");
        println!("List information about the FILEs (the current directory by default).");
        println!("\nOptions:");
        print!("{}", self.flags.help());
    }
}

impl Exec for Dir {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let opts = self.parse_args(scope, args)?;
        if opts.help {
            self.print_help();
            return Ok(Value::success());
        }

        list_entries(scope, &opts, &args)
    }
}

#[cfg(windows)]
mod win {
    use super::OWNER_MAX_LEN;
    use super::*;
    use std::cmp::min;
    use std::fs::{self, OpenOptions};
    use std::os::windows::prelude::*;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSidToSidW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        LookupAccountSidW, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SID_NAME_USE,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    use windows_sys::Win32::Foundation::LocalFree;

    /// Return a pair of Option<String> for the names of the owner and the group.
    fn get_owner_and_group_sids(
        mut path: PathBuf,
        metadata: &fs::Metadata,
    ) -> (Option<String>, Option<String>) {
        let get_sid_string = |psid: PSID| unsafe {
            let mut sid_string_ptr = PWSTR::null();

            if ConvertSidToStringSidW(psid, &mut sid_string_ptr).is_ok() {
                let sid_string = sid_string_ptr
                    .to_string()
                    .unwrap_or_else(|_| "?".to_string());

                LocalFree(sid_string_ptr.0 as _);
                Some(sid_string)
            } else {
                None
            }
        };

        if metadata.is_symlink() {
            // TODO: command line flag for following symlinks? My default use-case is ON.
            match resolve_links(&path) {
                Ok(p) => path = p,
                Err(_) => return (None, None),
            }
        }

        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
            .open(&path)
        {
            Ok(file) => file,
            Err(_) => return (None, None),
        };

        let handle = HANDLE(file.as_raw_handle());

        unsafe {
            let mut psid_owner = PSID::default();
            let mut psid_group = PSID::default();
            let mut sd = PSECURITY_DESCRIPTOR::default();

            let result = GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION,
                Some(&mut psid_owner),
                Some(&mut psid_group),
                None,
                None,
                Some(&mut sd),
            );

            if result.0 == 0 {
                let owner = get_sid_string(psid_owner);
                let group = get_sid_string(psid_group);

                LocalFree(sd.0);

                (owner, group)
            } else {
                (None, None)
            }
        }
    }

    /// Convert the SID string to an account name.
    fn name_from_sid(opt_sid: Option<String>) -> String {
        if let Some(sid) = opt_sid {
            unsafe {
                let mut psid = PSID::default();
                let wide_sid: Vec<u16> = sid.encode_utf16().chain(std::iter::once(0)).collect();

                if ConvertStringSidToSidW(PCWSTR(wide_sid.as_ptr()), &mut psid).is_err() {
                    return sid[..OWNER_MAX_LEN].to_string();
                }

                let mut name_size: u32 = 0;
                let mut domain_size: u32 = 0;
                let mut sid_use = SID_NAME_USE::default();

                // First call to get buffer sizes, ignore result
                _ = LookupAccountSidW(
                    PWSTR::null(),
                    psid,
                    PWSTR::null(),
                    &mut name_size,
                    PWSTR::null(),
                    &mut domain_size,
                    &mut sid_use,
                );

                let mut name = vec![0u16; name_size as usize];
                let mut domain = vec![0u16; domain_size as usize];

                // Second call to get actual data
                let success = LookupAccountSidW(
                    PWSTR::null(),
                    psid,
                    PWSTR(name.as_mut_ptr()),
                    &mut name_size,
                    PWSTR(domain.as_mut_ptr()),
                    &mut domain_size,
                    &mut sid_use,
                )
                .is_ok();

                LocalFree(psid.0);

                if success {
                    name_size = min(name_size, OWNER_MAX_LEN as u32);
                    String::from_utf16_lossy(&name[..name_size as usize])
                } else {
                    sid[..OWNER_MAX_LEN].to_string()
                }
            }
        } else {
            "-".to_string()
        }
    }

    pub fn get_owner_and_group(path: PathBuf, metadata: &fs::Metadata) -> (String, String) {
        let (owner_sid, group_sid) = get_owner_and_group_sids(path, metadata);
        (name_from_sid(owner_sid), name_from_sid(group_sid))
    }

    pub fn get_permissions(metadata: &fs::Metadata) -> String {
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
}

#[cfg(unix)]
fn get_owner_and_group(_: PathBuf, metadata: &fs::Metadata) -> (String, String) {
    use std::os::unix::fs::MetadataExt;
    use uzers::{get_group_by_gid, get_user_by_uid};

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

#[cfg(not(any(unix, windows)))]
fn get_permissions(_metadata: &fs::Metadata) -> String {
    "---------".to_string()
}

#[cfg(not(any(unix, windows)))]
fn get_owner_and_group(_: PathBuf, _: &fs::Metadata) -> (String, String) {
    ("-".to_string(), "-".to_string())
}

#[cfg(windows)]
use win::{get_owner_and_group, get_permissions};

fn list_entries(scope: &Rc<Scope>, opts: &Options, args: &Vec<String>) -> Result<Value, String> {
    for entry_path in &opts.paths {
        let mut path = PathBuf::from(entry_path);
        if path.is_symlink() {
            // TODO: command line options to follow symlinks specified on the cmd line?
            path = resolve_links(&path)?;
        }

        path = path
            .canonicalize()
            .map_err(|e| format!("{}: {}", scope.err_path_arg(&entry_path, args), e))?;

        match fs::metadata(&path) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    print_dir(scope, &path, &opts)?;
                } else {
                    print_file(&path, &metadata, &opts)?;
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(Value::success())
}

fn print_dir(scope: &Rc<Scope>, path: &Path, args: &Options) -> Result<(), String> {
    let entries =
        fs::read_dir(path).map_err(|e| format!("Cannot access {}: {}", path.display(), e))?;

    let mut entries: Vec<_> = entries
        .collect::<Result<_, _>>()
        .map_err(|e| format!("Error reading entries: {}", e))?;

    entries.sort_by_key(|e| e.file_name());

    if args.paths.len() > 1 {
        my_println!("\n{}:", path.display())?;
    }

    if args.show_details {
        print_detailed_entries(scope, &entries, &args)?;
    } else {
        print_simple_entries(&entries, &args, 4)?;
    }
    Ok(())
}

fn print_file(path: &Path, metadata: &Metadata, args: &Options) -> Result<(), String> {
    if args.show_details {
        print_details(&PathBuf::from(path), metadata, args)?;
    } else if args.all_files || !path.starts_with(".") {
        my_println!(
            "{}",
            args.colors
                .render_file_name(path.to_str().unwrap_or(""), metadata)
        )?;
    }
    Ok(())
}

fn print_simple_entries(
    entries: &Vec<DirEntry>,
    args: &Options,
    spacing: usize,
) -> Result<(), String> {
    let max_width = entries
        .iter()
        .filter(|e| args.all_files || !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.file_name().to_string_lossy().len())
        .max()
        .unwrap_or(0);

    let column_width = max_width + spacing;
    let terminal_width = terminal_size().map(|(Width(w), _)| w).unwrap_or(80) as usize;
    let columns = std::cmp::max(1, terminal_width / column_width);
    let mut current_column = 0;

    for entry in entries.iter() {
        let file_name = entry.file_name().to_string_lossy().to_string();

        if !args.all_files && file_name.starts_with('.') {
            continue;
        }

        if current_column >= columns {
            my_println!("{}", "")?;
            current_column = 0;
        }

        let file_name = match entry.metadata() {
            Ok(metadata) => args.colors.render_file_name(&file_name, &metadata),
            Err(_) => args.colors.render_error_path(&entry.path()),
        };

        if current_column == 0 {
            my_print!("{:<width$}", file_name, width = column_width)?;
        } else {
            my_print!(
                " {:<width$}",
                file_name,
                width = column_width.saturating_sub(1)
            )?;
        }

        current_column += 1;
    }

    if current_column != 0 {
        my_println!("{}", "")?;
    }

    Ok(())
}

fn print_detailed_entries(
    scope: &Rc<Scope>,
    entries: &Vec<DirEntry>,
    args: &Options,
) -> Result<(), String> {
    my_println!("total {}", entries.len())?;
    for entry in entries {
        match entry.metadata() {
            Ok(metadata) => {
                print_details(&entry.path(), &metadata, args)?;
            }
            Err(e) => {
                // Show warning and keep going.
                my_warning!(
                    scope,
                    "Cannot access {}: {}",
                    args.colors.render_error_path(&entry.path()),
                    args.colors.render_error(&e)
                );
                let unknown = "-";
                my_println!(
                    "-?????????  {0:OWNER_MAX_LEN$} {0:OWNER_MAX_LEN$} {1:>12}  {1:>12}  {2}",
                    unknown,
                    "?",
                    args.colors.render_error_path(&entry.path())
                )?;
            }
        }
    }
    Ok(())
}

/// Print details for one file entry
fn print_details(path: &Path, metadata: &Metadata, args: &Options) -> Result<(), String> {
    let base_name = path
        .file_name()
        .or(Some(path.as_os_str()))
        .unwrap()
        .to_string_lossy();

    if args.all_files || !base_name.starts_with(".") {
        let file_name = if metadata.is_symlink() {
            let link_path = resolve_links(path).unwrap_or(PathBuf::from("[...]"));
            format!("{} -> {}", base_name, link_path.display())
        } else {
            base_name.to_string()
        };

        let modified_time = format_time(metadata.modified().unwrap_or(UNIX_EPOCH), args.use_utc);
        let (owner, group) = get_owner_and_group(PathBuf::from(path), &metadata);

        my_println!(
            "{}{}  {:OWNER_MAX_LEN$} {:OWNER_MAX_LEN$} {:>12}  {}  {}",
            args.colors.render_file_type(format_file_type(&metadata)),
            args.colors.render_permissions(get_permissions(&metadata)),
            owner,
            group,
            args.colors.render_size(path, file_size(&metadata, args)),
            args.colors.render_mod_time(modified_time),
            args.colors.render_file_name(&file_name, metadata)
        )?;
    }
    Ok(())
}

fn file_size(metadata: &Metadata, args: &Options) -> String {
    if metadata.is_dir() {
        String::default()
    } else {
        format_size(metadata.len(), 1, args.human_readable)
    }
}

fn format_file_type(metadata: &fs::Metadata) -> &'static str {
    if metadata.is_dir() {
        "d"
    } else if metadata.is_file() {
        "-"
    } else if metadata.is_symlink() {
        "l"
    } else {
        "?"
    }
}

fn format_time(time: SystemTime, use_utc: bool) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    if let Some(datetime) = DateTime::from_timestamp(duration.as_secs() as i64, 0) {
        let formatted = if use_utc {
            datetime.with_timezone(&Utc).format("%b %d %H:%M")
        } else {
            datetime.with_timezone(&Local).format("%b %d %H:%M")
        };
        formatted.to_string()
    } else {
        "?".to_owned()
    }
}

#[ctor::ctor]
fn register() {
    let exec = Rc::new(Dir::new());

    register_command(ShellCommand {
        name: "ls".to_string(),
        inner: Rc::clone(&exec) as Rc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "dir".to_string(),
        inner: Rc::clone(&exec) as Rc<dyn Exec>,
    });
}
