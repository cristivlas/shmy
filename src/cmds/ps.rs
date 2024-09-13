use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::fmt;
use std::sync::Arc;
use sysinfo::{Pid, Process, System, Uid, Users};

trait Filter {
    fn apply<'a>(&self, proc: &'a Process) -> Option<&'a Process>;
}

/// Column formatter.
type Fmt = Box<dyn Fn(&mut fmt::Formatter<'_>, &dyn fmt::Display) -> fmt::Result>;

trait Field {
    fn to_string(&self, fmt: &Fmt) -> String;
}

/// Generic impl of a column in the processes view.
struct Column<G, T>
where
    G: Fn(&Process) -> T,
    T: Field,
{
    name: String,
    fmt: Fmt,
    getter: G,
}

impl<G, T> Column<G, T>
where
    G: Fn(&Process) -> T,
    T: Field + 'static,
{
    fn new(name: &str, fmt: Fmt, getter: G) -> Self {
        Self {
            name: name.to_string(),
            fmt,
            getter,
        }
    }
}

struct Header<'a> {
    col: &'a dyn ViewColumn,
}

impl<'a> fmt::Display for Header<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.col.fmt(f, &self.col.name())
    }
}

/// The interface for a column in the processes view.
trait ViewColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>, d: &dyn fmt::Display) -> fmt::Result;
    fn field(&self, proc: &Process) -> String;
    fn header(&self) -> Header;
    fn name(&self) -> &str;
}

impl<G, T> ViewColumn for Column<G, T>
where
    G: Fn(&Process) -> T,
    T: Field + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>, d: &dyn fmt::Display) -> fmt::Result {
        (self.fmt)(f, d)
    }

    fn field(&self, proc: &Process) -> String {
        let field = Box::new((self.getter)(proc));
        field.to_string(&self.fmt)
    }

    fn header(&self) -> Header<'_> {
        Header { col: self }
    }

    fn name(&self) -> &str {
        &self.name
    }
}

///
/// Field formatters
///
/// Define a Helper wrapper struct to have something to implement fmt::Display for.
struct Helper<'a, T: fmt::Display> {
    data: T,
    fmt: &'a Fmt,
}

impl<'a, T: fmt::Display> Helper<'a, T> {
    fn new(data: T, fmt: &'a Fmt) -> Self {
        Self { data, fmt }
    }
}

/// Implement Display by delegating to the fmt custom formmatter closure.
impl<'a, T: fmt::Display> fmt::Display for Helper<'a, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.fmt)(f, &self.data)
    }
}

impl Field for f32 {
    fn to_string(&self, fmt: &Fmt) -> String {
        Helper::new(format!("{:10.2}", self), fmt).to_string()
    }
}

impl Field for Pid {
    fn to_string(&self, fmt: &Fmt) -> String {
        Helper::new(self.as_u32(), fmt).to_string()
    }
}

impl Field for String {
    fn to_string(&self, fmt: &Fmt) -> String {
        let s = if self.len() > 30 { &self[..30] } else { &self };
        Helper::new(s, fmt).to_string()
    }
}

///
/// Convert Uid to User.name and format for printing
///
use std::sync::OnceLock;
static USERS: OnceLock<Users> = OnceLock::new();

fn get_users() -> &'static Users {
    USERS.get_or_init(|| Users::new_with_refreshed_list())
}

impl Field for Option<Uid> {
    fn to_string(&self, fmt: &Fmt) -> String {
        match self {
            Some(uid) => match get_users().iter().find(|user| user.id() == uid) {
                Some(user) => Helper::new(user.name(), fmt).to_string(),
                None => Helper::new(uid.to_string(), fmt).to_string(),
            },
            None => Helper::new("", fmt).to_string(),
        }
    }
}

/// Filters
///
/// Filter for including only processes belonging to the user running this command.
struct UserProc {
    uid: Option<Uid>,
}

impl UserProc {
    fn new(system: &System) -> Self {
        let uid = match sysinfo::get_current_pid() {
            Ok(pid) => system.process(pid).and_then(|p| p.user_id()).cloned(),
            Err(e) => {
                eprintln!("{}", e);
                None
            }
        };

        Self { uid }
    }
}

impl Filter for UserProc {
    fn apply<'a>(&self, proc: &'a Process) -> Option<&'a Process> {
        if self.uid.is_none() {
            Some(proc)
        } else {
            match proc.user_id() {
                Some(uid) => {
                    if *uid == *self.uid.as_ref().unwrap() {
                        Some(proc)
                    } else {
                        None
                    }
                }
                None => None,
            }
        }
    }
}

struct View {
    columns: Vec<Box<dyn ViewColumn>>,
    filters: Vec<Box<dyn Filter>>,
    system: System,
}

impl View {
    fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();

        Self {
            columns: vec![],
            filters: vec![],
            system,
        }
    }

    fn with_default_view() -> Self {
        let mut view = View::new();

        view.columns.push(Self::pid_column());
        view.columns.push(Self::user_column());
        view.columns.push(Self::name_column());
        view.columns.push(Self::cpu_usage_column());
        view.columns.push(Self::mem_usage_column());

        view
    }

    fn with_all_procs() -> Self {
        Self::with_default_view()
    }

    fn list_processes(&self) -> Result<(), String> {
        let mut header = String::new();

        for col in &self.columns {
            if !header.is_empty() {
                header.push_str("  ");
            }
            header.push_str(&col.header().to_string());
        }
        my_println!("{}", header)?;

        for (_, proc) in self.system.processes() {
            if self
                .filters
                .iter()
                .any(|filter| filter.apply(proc).is_none())
            {
                continue;
            }
            for col in &self.columns {
                my_print!("{}  ", col.field(proc))?;
            }
            my_println!()?;
        }
        Ok(())
    }

    fn cpu_usage_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "CPU%",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(Process::cpu_usage),
        ))
    }

    fn mem_usage_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "MEM (MB)",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(|p: &Process| p.memory() as f32 / 1024.0 / 1024.0),
        ))
    }

    fn name_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "NAME",
            Box::new(|f, d| write!(f, "{:<30}", d)),
            Box::new(|p: &Process| p.name().to_string_lossy().to_string()),
        ))
    }

    fn pid_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "PID",
            Box::new(|f, d| write!(f, "{:>8}", d)),
            Box::new(Process::pid),
        ))
    }

    fn user_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "USER",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(|p: &Process| p.user_id().map(|u| u.clone())),
        ))
    }
}

impl Default for View {
    fn default() -> Self {
        let mut view = Self::with_default_view();
        view.filters.push(Box::new(UserProc::new(&view.system)));

        view
    }
}

struct ProcStatus {
    flags: CommandFlags,
}

impl ProcStatus {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag(
            'a',
            "all",
            "List all processes, not just processes belonging to the current user",
        );

        Self { flags }
    }
}

impl Exec for ProcStatus {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let _ = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: ps [OPTIONS]");
            println!("List currently running processes and their details.");
            println!("\nOptions:");
            println!("{}", flags.help());
            return Ok(Value::success());
        }

        let view = if flags.is_present("all") {
            View::with_all_procs()
        } else {
            View::default()
        };
        view.list_processes()?;

        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "ps".to_string(),
        inner: Arc::new(ProcStatus::new()),
    });
}
