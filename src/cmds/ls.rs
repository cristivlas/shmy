use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::utils::{self, format_size, read_symlink, MAX_USER_DISPLAY_LEN};
use crate::{eval::Value, scope::Scope, symlnk::SymLink};
use chrono::{DateTime, Local, Utc};
use colored::*;
use core::fmt;
use std::fs::{self, DirEntry, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

struct ColorScheme {
    use_colors: bool,
    scope: Arc<Scope>,
}

impl ColorScheme {
    fn with_scope(scope: &Arc<Scope>) -> Self {
        Self {
            use_colors: scope.use_colors(&std::io::stdout()),
            scope: Arc::clone(&scope),
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

    fn render_size(&self, is_wsl_link: bool, size: String) -> ColoredString {
        let size = if is_wsl_link { "wsl" } else { &size };

        if self.use_colors {
            if is_wsl_link {
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

struct Options {
    all_files: bool,
    show_details: bool,
    human_readable: bool,
    help: bool,
    paths: Vec<String>,
    colors: ColorScheme,
    utc: bool,       // show file times in UTC
    base_name: bool, // Use base name only with -l/--long listing
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

    fn parse_args(&self, scope: &Arc<Scope>, args: &[String]) -> Result<Options, String> {
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
            utc: flags.is_present("utc"),
            base_name: false,
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
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut opts = self.parse_args(scope, args)?;
        if opts.help {
            self.print_help();
            return Ok(Value::success());
        }

        list_entries(scope, &mut opts, &args)
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use crate::utils::win::name_from_sid;
    use std::fs::{self, OpenOptions};
    use std::os::windows::prelude::*;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, GetSecurityInfo, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
    use windows_sys::Win32::Foundation::LocalFree;

    /// Return a pair of Option<String> for the names of the owner and the group.
    fn get_owner_and_group_sids(path: &Path) -> (Option<String>, Option<String>) {
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

    pub fn get_owner_and_group(path: &Path, _: &fs::Metadata) -> (String, String) {
        let (owner_sid, group_sid) = get_owner_and_group_sids(path);
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
fn get_owner_and_group(_: &Path, metadata: &fs::Metadata) -> (String, String) {
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
fn get_owner_and_group(_: &Path, _: &fs::Metadata) -> (String, String) {
    ("-".to_string(), "-".to_string())
}

#[cfg(windows)]
use win::{get_owner_and_group, get_permissions};

fn list_entries(
    scope: &Arc<Scope>,
    opts: &mut Options,
    args: &Vec<String>,
) -> Result<Value, String> {
    for entry_path in &opts.paths {
        let path = Path::new(entry_path)
            .resolve()
            .map_err(|e| format!("{}: {}", scope.err_path_arg(&entry_path, args), e))?;

        match fs::metadata(&path) {
            Ok(metadata) => {
                if metadata.is_dir() {
                    opts.base_name = true;
                    print_dir(scope, &path, &opts)?;
                } else {
                    opts.base_name = false;
                    print_file(&path, &metadata, &opts)?;
                }
            }
            Err(e) => {
                return Err(format!("{}: {}", scope.err_path_arg(&entry_path, args), e));
            }
        }
    }

    Ok(Value::success())
}

fn print_dir(scope: &Arc<Scope>, path: &Path, args: &Options) -> Result<(), String> {
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
        let name = path.canonicalize().map_err(|e| e.to_string())?;
        my_println!(
            "{}",
            args.colors
                .render_file_name(&name.to_string_lossy().to_string(), metadata)
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
    let terminal_width = utils::terminal_width();
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
    scope: &Arc<Scope>,
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
                    "-?????????  {0:MAX_USER_DISPLAY_LEN$} {0:MAX_USER_DISPLAY_LEN$} {1:>12}  {1:>12}  {2}",
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
fn print_details(path: &Path, metadata: &Metadata, opts: &Options) -> Result<(), String> {
    let file_name = if opts.base_name {
        path.file_name()
            .or(Some(path.as_os_str()))
            .unwrap()
            .to_string_lossy()
    } else {
        path.to_string_lossy()
    };

    let (is_wsl, real_path) = if path.is_wsl_link().unwrap_or(false) {
        (true, read_symlink(path).unwrap_or_default())
    } else {
        (
            false,
            if path.is_symlink() {
                path.read_link().unwrap_or(PathBuf::from("[...]"))
            } else {
                path.to_path_buf()
            },
        )
    };

    if opts.all_files || !file_name.starts_with(".") {
        let file_name = if metadata.is_symlink() {
            let link_path = &real_path;
            format!("{} -> {}", file_name, link_path.display())
        } else {
            file_name.to_string()
        };

        let modified_time = format_time(metadata.modified().unwrap_or(UNIX_EPOCH), opts.utc);
        let (owner, group) = get_owner_and_group(&real_path, &metadata);

        my_println!(
            "{}{}  {:MAX_USER_DISPLAY_LEN$} {:MAX_USER_DISPLAY_LEN$} {:>12}  {}  {}",
            opts.colors.render_file_type(format_file_type(&metadata)),
            opts.colors.render_permissions(get_permissions(&metadata)),
            owner,
            group,
            opts.colors.render_size(is_wsl, file_size(&metadata, opts)),
            opts.colors.render_mod_time(modified_time),
            opts.colors.render_file_name(&file_name, metadata)
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
    let exec = Arc::new(Dir::new());

    register_command(ShellCommand {
        name: "ls".to_string(),
        inner: Arc::clone(&exec) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "dir".to_string(),
        inner: Arc::clone(&exec) as Arc<dyn Exec>,
    });
}
