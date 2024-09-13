use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope};
use std::fmt;
use std::sync::Arc;
use sysinfo::{Pid, Process, System, Uid};

type Fmt = Box<dyn Fn(&mut fmt::Formatter<'_>, &dyn fmt::Display) -> fmt::Result>;

trait Field {
    fn to_string(&self, fmt: &Fmt) -> String;
}

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

struct View {
    columns: Vec<Box<dyn ViewColumn>>,
}

impl View {
    fn new() -> Self {
        Self { columns: vec![] }
    }

    fn list_processes(&self, system: &System) {
        let mut header = String::new();

        for col in &self.columns {
            if !header.is_empty() {
                header.push_str("  ");
            }
            header.push_str(&col.header().to_string());
        }
        println!("{}", header);

        for (_, proc) in system.processes() {
            for col in &self.columns {
                print!("{}  ", col.field(proc));
            }
            println!()
        }
    }
}

struct Helper<'a, T: fmt::Display> {
    data: T,
    fmt: &'a Fmt,
}

impl<'a, T: fmt::Display> Helper<'a, T> {
    fn new(data: T, fmt: &'a Fmt) -> Self {
        Self { data, fmt }
    }
}

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
        Helper::new(self, fmt).to_string()
    }
}

impl Field for Option<Uid> {
    fn to_string(&self, fmt: &Fmt) -> String {
        match self {
            Some(uid) => Helper::new(uid.to_string(), fmt).to_string(),
            _ => String::default(),
        }
    }
}

impl Default for View {
    fn default() -> Self {
        let mut view = View::new();

        view.columns.push(Box::new(Column::new(
            "PID",
            Box::new(|f, d| write!(f, "{:>8}", d)),
            Box::new(Process::pid),
        )));

        view.columns.push(Box::new(Column::new(
            "NAME",
            Box::new(|f, d| write!(f, "{:<40}", d)),
            Box::new(|p: &Process| p.name().to_string_lossy().to_string()),
        )));

        view.columns.push(Box::new(Column::new(
            "CPU%",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(Process::cpu_usage),
        )));

        view.columns.push(Box::new(Column::new(
            "MEM (MB)",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(|p: &Process| p.memory() as f32 / 1024.0 / 1024.0),
        )));

        view.columns.push(Box::new(Column::new(
            "USER",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(|p: &Process| p.user_id().map(|u| u.clone())),
        )));

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

        Self { flags }
    }

    fn list_processes(&self) -> Result<(), String> {
        let mut system = System::new_all();
        system.refresh_all();

        View::default().list_processes(&system);

        Ok(())
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

        self.list_processes()?;

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
