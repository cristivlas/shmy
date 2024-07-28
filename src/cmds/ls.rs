use super::{register_command, BuiltinCommand, Exec};
use crate::eval::Value;
use chrono::DateTime;
use std::fs;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

struct Dir;

impl Exec for Dir {
    fn exec(&self, args: Vec<String>) -> Result<Value, String> {
        let (details, megs, specs) = parse_args(&args);
        let paths = if specs.is_empty() {
            vec![".".to_string()]
        } else {
            specs
        };

        list_directories(&paths, details, megs)
    }
}

fn parse_args(args: &[String]) -> (bool, bool, Vec<String>) {
    let mut details = false;
    let mut megs = false;
    let mut specs = Vec::new();

    for arg in args {
        if arg.starts_with('-') {
            for flag in arg.chars().skip(1) {
                match flag {
                    'l' => details = true,
                    'h' => megs = true,
                    _ => {
                        eprintln!("Unknown flag ignored: {}", flag);
                    }
                }
            }
        } else {
            specs.push(arg.clone());
        }
    }

    (details, megs, specs)
}

fn format_file_type(metadata: &fs::Metadata) -> char {
    if metadata.is_dir() {
        'd'
    } else if metadata.is_file() {
        '-'
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
fn get_owner_and_group(metadata: &fs::Metadata) -> (String, String) {
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

#[cfg(not(unix))]
fn get_owner_and_group(_metadata: &fs::Metadata) -> (String, String) {
    ("unknown".to_string(), "unknown".to_string())
}

fn list_directories(paths: &[String], details: bool, megs: bool) -> Result<Value, String> {
    for path in paths {
        let entries = fs::read_dir(path).map_err(|e| format!("cannot access '{}': {}", path, e))?;

        let mut entries: Vec<_> = entries
            .collect::<Result<_, _>>()
            .map_err(|e| format!("Error reading entries: {}", e))?;

        entries.sort_by_key(|e| e.file_name());

        if details {
            println!("total {}", entries.len());
            for entry in entries {
                let metadata = entry
                    .metadata()
                    .map_err(|e| format!("Failed to get metadata: {}", e))?;
                let file_name = entry.file_name();
                let size = if megs {
                    format!("{:.2}M", metadata.len() as f64 / 1_048_576.0)
                } else {
                    metadata.len().to_string()
                };
                let file_type = format_file_type(&metadata);
                let modified_time = format_time(metadata.modified().unwrap_or(UNIX_EPOCH));
                let (owner, group) = get_owner_and_group(&metadata);
                println!(
                    "{} {:>8} {:>8} {:>8} {:>16} {}",
                    file_type,
                    owner,
                    group,
                    size,
                    modified_time,
                    file_name.to_string_lossy()
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
    register_command(BuiltinCommand {
        name: "ls",
        exec: Rc::new(Dir),
    });
}
