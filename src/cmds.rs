use crate::utils::{copy_vars_to_command_env, wait_child};
use crate::{eval::Value, scope::Scope};
use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::Debug;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use which::which;

mod flags;
use flags::CommandFlags;
// Built-in commands
mod alias;
mod basename;
mod cat;
mod cd;
mod chmod;
mod clear;
mod cp;
mod cut;
mod date;
mod defined;
#[cfg(windows)]
mod df;
mod diff;
mod du;
mod echo;
mod evalargs;
mod exit;
mod find;
mod grep;
mod help;
mod less;
mod ln;
mod ls;
mod mkdir;
mod mv;
mod open;
#[cfg(windows)]
mod power;
mod ps;
mod realpath;
mod rm;
mod run;
mod sort;
mod strings;
#[cfg(windows)]
mod sudo;
mod touch;
mod vars;
mod wc;
#[cfg(windows)]
mod whois;

pub trait Exec {
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }

    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String>;

    /// Return true if command needs a shell to launch.
    #[allow(dead_code)]
    fn is_script(&self) -> bool {
        false
    }

    fn path(&self) -> Cow<'_, Path> {
        unreachable!()
    }

    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(std::iter::empty())
    }
}

#[derive(Clone)]
pub struct Flag {
    pub short: Option<char>,
    pub long: String,
    pub help: String,
    pub takes_value: Option<String>,
    pub default_value: Option<String>,
}

#[derive(Clone)]
pub struct ShellCommand {
    name: String,
    inner: Arc<dyn Exec>,
}

impl ShellCommand {
    pub fn name(&self) -> &String {
        &self.name
    }

    fn is_alias(&self) -> bool {
        self.inner
            .as_ref()
            .as_any()
            .and_then(|any| any.downcast_ref::<alias::AliasRunner>())
            .is_some()
    }

    fn is_external(&self) -> bool {
        self.inner
            .as_ref()
            .as_any()
            .and_then(|any| any.downcast_ref::<External>())
            .is_some()
    }
}

impl Debug for ShellCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "name: {}", &self.name)
    }
}

impl Exec for ShellCommand {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        self.inner.cli_flags()
    }

    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        self.inner.exec(name, args, scope)
    }

    fn is_script(&self) -> bool {
        self.inner.is_script()
    }

    fn path(&self) -> Cow<'_, Path> {
        self.inner.path()
    }
}

unsafe impl Send for ShellCommand {}

static COMMAND_REGISTRY: LazyLock<Mutex<HashMap<String, ShellCommand>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn register_command(command: ShellCommand) -> Option<ShellCommand> {
    COMMAND_REGISTRY
        .lock()
        .unwrap()
        .insert(command.name.clone(), command)
}

pub fn unregister_command(name: &str) {
    COMMAND_REGISTRY.lock().unwrap().remove(name);
}

pub fn get_command(name: &str) -> Option<ShellCommand> {
    let mut cmd = COMMAND_REGISTRY.lock().unwrap().get(name).cloned();
    if cmd.is_none() {
        if let Some(_) = which_executable(Path::new(name)) {
            // Do not cache the path, as $PATH may change later.
            register_command(ShellCommand {
                name: name.to_string(),
                inner: Arc::new(External {
                    path: PathBuf::from(name),
                }),
            });
            cmd = COMMAND_REGISTRY.lock().unwrap().get(name).cloned();
        }
    }

    cmd
}

pub fn registered_commands(internal_only: bool) -> Vec<String> {
    let registry = COMMAND_REGISTRY.lock().unwrap();

    let mut commands: Vec<String> = if internal_only {
        registry
            .keys()
            .cloned()
            .filter(|k| registry.get(k).map_or(true, |c| !c.is_external()))
            .collect()
    } else {
        registry.keys().cloned().collect()
    };
    commands.sort();
    commands
}

fn which_executable<T: AsRef<OsStr>>(path: T) -> Option<PathBuf> {
    match which(path) {
        Ok(path) => {
            // Check if the path is an executable
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.is_file() && is_executable(&path) {
                    return Some(path);
                }
            }
            None
        }
        Err(_) => None,
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    // Check the file's executable permission
    use std::os::unix::fs::PermissionsExt;
    if let Ok(perms) = fs::metadata(path).map(|m| m.permissions()) {
        perms.mode() & 0o111 != 0 // Check if any execute bit is set
    } else {
        false
    }
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    // On Windows, check if the file extension is in PATHEXT
    if let Some(ext) = path.extension().and_then(std::ffi::OsStr::to_str) {
        let pathext = std::env::var("PATHEXT").unwrap_or_default();
        let ext_lower = format!(".{}", ext).to_lowercase();
        let mut extensions = pathext.split(';');

        extensions.any(|e| e.eq_ignore_ascii_case(&ext_lower))
    } else {
        false
    }
}

// Wrap execution of an external program.
struct External {
    path: PathBuf,
}

impl External {
    fn which_path(&self) -> Cow<'_, Path> {
        if self.path.is_absolute() {
            Cow::Borrowed(&self.path)
        } else if let Some(path) = which_executable(&self.path) {
            Cow::Owned(path)
        } else {
            Cow::Borrowed(&self.path)
        }
    }
}

#[cfg(unix)]
impl External {
    fn prepare_command(&self, args: &Vec<String>) -> Command {
        let mut command = Command::new(self.path().as_os_str());
        command.args(args);
        command
    }
}

#[cfg(windows)]
impl External {
    fn prepare_command(&self, args: &Vec<String>) -> Command {
        let path = self.which_path();
        if self.is_script() {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(path.as_os_str()).args(args);
            command
        } else {
            let mut command = Command::new(path.as_os_str());
            command.args(args);
            command
        }
    }
}

impl Exec for External {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut command = self.prepare_command(args);
        copy_vars_to_command_env(&mut command, &scope);

        match command.spawn() {
            Ok(mut child) => match &wait_child(&mut child) {
                Ok(status) => {
                    if let Some(code) = status.code() {
                        if code != 0 {
                            return Err(format!("exit code: {}", code));
                        }
                    }
                    return Ok(Value::success());
                }
                Err(e) => Err(format!("Failed to wait on child process: {}", e)),
            },
            Err(e) => Err(format!("Failed to execute command: {}", e)),
        }
    }

    /// External commands that are not EXEs are launched via CMD.EXE
    /// This is a simpler approach than looking up file associations
    /// in the registry.
    #[cfg(windows)]
    fn is_script(&self) -> bool {
        let ext = self
            .which_path()
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_owned();

        !matches!(ext.as_str(), "exe")
    }

    /// Looks like (at least on Linux) the shebang just works
    /// and there is no need for special handling of scripts.
    #[cfg(unix)]
    fn is_script(&self) -> bool {
        false
    }

    fn path(&self) -> Cow<'_, Path> {
        self.which_path()
    }
}

struct Which {
    flags: CommandFlags,
}

impl Which {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('e', "external", "Show external commands only");
        Which { flags }
    }
}

impl Exec for Which {
    fn cli_flags(&self) -> Box<dyn Iterator<Item = &Flag> + '_> {
        Box::new(self.flags.iter())
    }

    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: which [COMMAND]...");
            println!("Locate a command and display its path.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if args.is_empty() {
            return Err("which: missing command name".to_string());
        }

        let extern_only = flags.is_present("external");

        for command in args {
            if let Some(cmd) = get_command(command) {
                if !extern_only && !cmd.is_external() {
                    if cmd.is_alias() {
                        my_println!("{}: alias", command)?;
                    } else {
                        my_println!("{}: built-in", command)?;
                    }
                }
            }
            if let Some(path) = which_executable(command) {
                my_println!("{}", path.display())?;
            }
        }

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "which".to_string(),
        inner: Arc::new(Which::new()),
    });
}
