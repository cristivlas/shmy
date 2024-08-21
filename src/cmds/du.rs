use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::utils::format_size;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct DiskUtilization {
    flags: CommandFlags,
}

impl DiskUtilization {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('s', "summarize", "Display only a total for each argument");
        flags.add_flag(
            'h',
            "human-readable",
            "Print sizes in human readable format (e.g., 1.1K, 234M, 2.7G)",
        );
        Self { flags }
    }
}

struct Options {
    human: bool,
    summarize: bool,
    block_size: u64,
}

impl Exec for DiskUtilization {
    fn is_external(&self) -> bool {
        false
    }

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
            summarize: flags.is_present("summarize"),
            human: flags.is_present("human-readable"),
            block_size: 1024,
        };

        let mut total_size: u64 = 0;

        for p in &paths {
            let path = PathBuf::from(p);
            let size = du_size(&path, &opts, scope)?;
            total_size += size;
        }

        if opts.summarize {
            my_println!("{}", format_size(total_size, opts.block_size, opts.human))?;
        }

        Ok(Value::success())
    }
}

fn du_size(path: &Path, opts: &Options, scope: &Rc<Scope>) -> Result<u64, String> {
    if path.is_symlink() {
        return Ok(0);
    }

    let mut size: u64 = 0;
    size += path_size(scope, path)?;

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
                    size += du_size(&entry.path(), &opts, scope)?;
                }

                if !opts.summarize {
                    my_println!(
                        "{}\t{}",
                        format_size(size, opts.block_size, opts.human),
                        path.display()
                    )?;
                }
            }
        }
    }

    Ok(size)
}

fn path_size(scope: &Rc<Scope>, path: &Path) -> Result<u64, String> {
    Ok(fs::metadata(path)
        .map_err(|e| format!("{}: {}", scope.err_path(path), e))?
        .len())
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "du".to_string(),
        inner: Rc::new(DiskUtilization::new()),
    });
}
