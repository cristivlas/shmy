use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{
    eval::Value,
    scope::Scope,
    utils::{format_error, MAX_USER_DISPLAY_LEN},
};
use std::{
    any::Any,
    cmp::{Ord, Ordering, PartialOrd},
    collections::BTreeMap,
    collections::BTreeSet,
    collections::HashMap,
    collections::HashSet,
    ffi::OsStr,
    ffi::OsString,
    fmt,
    sync::Arc,
};
use sysinfo::{Pid, Process, System, Uid};

const MAX_STR_WIDTH: usize = 32;

trait Filter {
    fn apply<'a>(&self, proc: &'a Process) -> Option<&'a Process>;
}

/// Column formatter.
type Fmt = Box<dyn Fn(&mut fmt::Formatter<'_>, &dyn fmt::Display) -> fmt::Result>;

trait Field {
    fn as_any(&self) -> &dyn Any;
    fn to_string(&self, fmt: &Fmt) -> String;
}

/// Generic impl of a column in the processes view.
struct Column<G, T>
where
    G: Fn(&Process) -> T,
    T: Field,
{
    name: &'static str,
    header: &'static str,
    fmt: Fmt,
    getter: G,
}

impl<G, T> Column<G, T>
where
    G: Fn(&Process) -> T,
    T: Field + 'static,
{
    fn new(name: &'static str, header: &'static str, fmt: Fmt, getter: G) -> Self {
        Self {
            name,
            header,
            fmt,
            getter,
        }
    }
}

struct Header<'a> {
    col: &'a Box<dyn ViewColumn>,
}

impl<'a> Header<'a> {
    fn new(col: &'a Box<dyn ViewColumn>) -> Self {
        Self { col }
    }
}

impl<'a> fmt::Display for Header<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.col.fmt(f, &self.col.header())
    }
}

/// The interface for a column in the processes view.
trait ViewColumn {
    /// Order two processes based on this column.
    fn cmp(&self, lhs: &Process, rhs: &Process) -> Ordering;

    /// Format this column for printing
    fn fmt(&self, f: &mut fmt::Formatter<'_>, d: &dyn fmt::Display) -> fmt::Result;

    /// Extract field that corresponds to this column from process.
    fn field(&self, proc: &Process) -> Box<dyn Field>;

    /// Extract field and format at string, for printing.
    fn field_as_string(&self, proc: &Process) -> String;

    /// Get the printable column header.
    fn header(&self) -> &str;

    /// Get the column's name.
    fn name(&self) -> &'static str;
}

impl<G, T> ViewColumn for Column<G, T>
where
    G: Fn(&Process) -> T,
    T: Field + Ord + 'static,
{
    fn cmp(&self, lhs: &Process, rhs: &Process) -> Ordering {
        if let (Some(lhs), Some(rhs)) = (
            self.field(lhs).as_any().downcast_ref::<T>(),
            self.field(rhs).as_any().downcast_ref::<T>(),
        ) {
            lhs.cmp(rhs)
        } else {
            // Types don't match or can't be compared? Fall back to string comparison.
            self.field_as_string(lhs).cmp(&self.field_as_string(rhs))
        }
    }

    fn fmt(&self, f: &mut fmt::Formatter<'_>, d: &dyn fmt::Display) -> fmt::Result {
        (self.fmt)(f, d)
    }

    fn field(&self, proc: &Process) -> Box<dyn Field> {
        Box::new((self.getter)(proc))
    }

    fn field_as_string(&self, proc: &Process) -> String {
        self.field(proc).to_string(&self.fmt)
    }

    fn header(&self) -> &str {
        &self.header
    }

    fn name(&self) -> &'static str {
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

fn truncate_and_pad(s: &str, width: usize) -> String {
    let truncated = if s.len() > width { &s[..width] } else { s };
    format!("{:>width$}", truncated, width = width)
}

/// Wrap f32 to define partial ordering for sorting by Cpu utilization,
/// memory, etc. f32 does not implement ordering by default, due to NaN.
#[derive(PartialEq, PartialOrd)]
struct F32(f32);

impl Eq for F32 {}

impl Ord for F32 {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or(Ordering::Less)
    }
}

impl Field for F32 {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        Helper::new(format!("{:10.2}", self.0), fmt).to_string()
    }
}

impl Field for Pid {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        Helper::new(self.as_u32(), fmt).to_string()
    }
}

impl Field for Option<Pid> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        match self {
            Some(pid) => Helper::new(pid.as_u32(), fmt).to_string(),
            None => Helper::new(0, fmt).to_string(),
        }
    }
}

impl Field for String {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        Helper::new(truncate_and_pad(self, MAX_STR_WIDTH), fmt).to_string()
    }
}

/// OsString is used for showing the command that started a process.
/// Alternatively we could make a custom type, but this works.
impl Field for OsString {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        Helper::new(self.to_string_lossy(), fmt).to_string()
    }
}

#[derive(Eq, PartialEq, PartialOrd, Ord)]
struct RunTime(u64);

impl Field for RunTime {
    fn as_any(&self) -> &dyn Any {
        &self.0
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        let duration = std::time::Duration::from_secs(self.0);
        let days = duration.as_secs() / 86400;
        let hours = (duration.as_secs() % 86400) / 3600;
        let minutes = (duration.as_secs() % 3600) / 60;
        let seconds = duration.as_secs() % 60;

        // Format the duration
        let s = if days > 0 {
            format!("{}-{:02}:{:02}:{:02}", days, hours, minutes, seconds)
        } else {
            format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
        };
        // Apply column formatting, for alignment and width
        Helper::new(s, fmt).to_string()
    }
}

///
/// Convert Uid to User.name and format for printing
///
#[cfg(windows)]
fn uid_to_name(uid: &Uid) -> String {
    crate::utils::win::name_from_sid(Some(uid.to_string()))
}

#[cfg(not(windows))]
fn uid_to_name(uid: &Uid) -> String {
    use std::sync::OnceLock;
    use sysinfo::Users;
    static USERS: OnceLock<Users> = OnceLock::new();

    fn get_users() -> &'static Users {
        USERS.get_or_init(|| Users::new_with_refreshed_list())
    }

    match get_users().iter().find(|user| user.id() == uid) {
        Some(user) => user.name().to_string(),
        None => uid.to_string(),
    }
}

impl Field for Option<Uid> {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn to_string(&self, fmt: &Fmt) -> String {
        let s = match self {
            Some(uid) => uid_to_name(uid),
            None => String::default(),
        };

        Helper::new(truncate_and_pad(&s, MAX_USER_DISPLAY_LEN), fmt).to_string()
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

/// Sort children by name, depth and Pid
#[derive(Clone)]
struct TreeNode<'a> {
    children: BTreeSet<(&'a OsStr, usize, Pid)>,
}

impl<'a> Default for TreeNode<'a> {
    fn default() -> Self {
        Self {
            children: BTreeSet::new(),
        }
    }
}

struct View {
    columns: Vec<Box<dyn ViewColumn>>,
    filters: Vec<Box<dyn Filter>>,
    sort_keys: Vec<(&'static str, bool)>, // (name, reverse)
    system: System,
}

impl View {
    fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();

        Self {
            columns: vec![],
            filters: vec![],
            sort_keys: vec![],
            system,
        }
    }

    /// Display a list of running processes.
    fn process_list(&self) -> Result<(), String> {
        let mut header = String::new();

        for col in &self.columns {
            if !header.is_empty() {
                header.push_str("  ");
            }
            header.push_str(&Header::new(col).to_string());
        }
        my_println!("{}", header)?;

        let mut processes: Vec<_> = self
            .system
            .processes()
            .iter()
            .map(|(_, p)| p)
            .filter_map(|p| {
                self.filters
                    .iter()
                    .fold(Some(p), |p, f| p.and_then(|p| f.apply(p)))
            })
            .collect();

        // Iterate over the sort keys in reverse, to ensure stable sort.
        for (k, reverse) in self.sort_keys.iter().rev() {
            if let Some(col) = self.columns.iter().find(|col| col.name() == *k) {
                processes.sort_by(|lhs, rhs| {
                    let ord = col.cmp(lhs, rhs);
                    if *reverse {
                        ord.reverse()
                    } else {
                        ord
                    }
                });
            }
        }

        for p in processes {
            for col in &self.columns {
                my_print!("{}  ", col.field_as_string(p))?;
            }
            my_println!()?;
        }
        Ok(())
    }

    /// Display processes in a tree-like, hierarchical view.
    fn process_tree(&mut self, long: bool) -> Result<(), String> {
        let mut roots = BTreeSet::new();
        let mut seen = HashSet::new(); // Cycles happen on Windows

        let processes = self.system.processes();

        // 1st pass: identify "roots"
        for proc in processes.values() {
            let mut child = proc;
            loop {
                match child.parent() {
                    None => {
                        roots.insert(child.pid());
                        break;
                    }
                    Some(parent_id) => {
                        if let Some(parent) = processes.get(&parent_id) {
                            if !seen.insert(child.pid()) {
                                roots.insert(parent_id);
                                break;
                            }
                            child = parent;
                        } else {
                            roots.insert(child.pid());
                            break;
                        }
                    }
                }
            }
        }

        // 2nd pass: construct a forest.
        let mut tree_map: BTreeMap<Pid, TreeNode> = BTreeMap::new();

        for proc in processes.values() {
            let mut child = proc;
            while !roots.contains(&child.pid()) {
                let parent_id = child.parent().expect("child with no parent");
                tree_map.entry(parent_id).or_default().children.insert((
                    child.name(),
                    0, // Place-holder, depth is updated in 3rd pass
                    child.pid(),
                ));

                child = processes
                    .get(&parent_id)
                    .expect("child with unknown parent");
            }
        }

        // 3rd pass: construct a copy of tree_map with updated depth tuple
        // elems in the key, so that children are sorted by (name, depth, pid)
        let mut depth_map = HashMap::new();
        for pid in processes.keys() {
            let depth = calculate_depth(pid, &tree_map);
            depth_map.insert(pid, depth);
        }

        let mut tree_depth_map = BTreeMap::new();

        for (pid, tree_node) in tree_map {
            let mut node = TreeNode::default();
            for (name, _, pid) in tree_node.children {
                let depth = *depth_map.get(&pid).unwrap();
                node.children.insert((name, depth, pid));
            }
            tree_depth_map.insert(pid, node);
        }

        for pid in &roots {
            print_tree(pid, long, &tree_depth_map, processes, "", false)?;
        }

        Ok(())
    }

    /// Parse a string containing sorting specification.
    /// The spec is a comma-separated list of column names, optionally prefixed by + or -
    /// + specifies increasing sort order (the default) and - specifies decreasing order.
    fn parse_sort_spec(
        &mut self,
        scope: &Arc<Scope>,
        sort_spec: &str,
        args: &Vec<String>,
    ) -> Result<(), String> {
        let mut seen = HashSet::new();

        for spec in sort_spec.split(',') {
            let (name, reverse) = match spec.trim() {
                s if s.starts_with('-') => (&s[1..], true),
                s if s.starts_with('+') => (&s[1..], false),
                s => (s, false), // default ascending
            };

            // Find the column by name and get the reference to the static `name`
            if let Some(col) = self.columns.iter().find(|col| col.name() == name) {
                let col_name = col.name(); // 'static reference
                if !seen.insert(col_name) {
                    return Err(format_error(
                        scope,
                        sort_spec,
                        args,
                        format!("Duplicate sort key: {}", col_name),
                    ));
                }
                self.sort_keys.push((col_name, reverse));
            } else {
                return Err(format_error(
                    scope,
                    sort_spec,
                    args,
                    format!("Invalid sort key: {}", name),
                ));
            }
        }

        Ok(())
    }

    //
    // Factory methods for ViewColumn-s. Columns are used in the default list view,
    // currently not used in the tree view.
    //
    fn cpu_usage_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "cpu",
            "CPU%",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(|proc: &Process| F32(proc.cpu_usage())),
        ))
    }

    fn cmd_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "cmd",
            "CMD",
            Box::new(|f, d| write!(f, "{:<}", d)),
            Box::new(|proc: &Process| cmd_string(proc)),
        ))
    }

    fn mem_usage_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "mem",
            "MEM (MB)",
            Box::new(|f, d| write!(f, "{:>10}", d)),
            Box::new(|proc: &Process| F32(proc.memory() as f32 / 1024.0 / 1024.0)),
        ))
    }

    fn name_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "name",
            "NAME",
            Box::new(|f, d| write!(f, "{:>MAX_STR_WIDTH$}", d)),
            Box::new(|p: &Process| p.name().to_string_lossy().to_string()),
        ))
    }

    fn parent_pid_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "ppid",
            "PPID",
            Box::new(|f, d| write!(f, "{:>8}", d)),
            Box::new(Process::parent),
        ))
    }

    fn pid_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "pid",
            "PID",
            Box::new(|f, d| write!(f, "{:>8}", d)),
            Box::new(Process::pid),
        ))
    }

    fn run_time_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "time",
            "TIME",
            Box::new(|f, d| write!(f, "{:>12}", d)),
            Box::new(|p: &Process| RunTime(p.run_time())),
        ))
    }

    fn user_column() -> Box<dyn ViewColumn> {
        Box::new(Column::new(
            "user",
            "USER",
            Box::new(|f, d| write!(f, "{:>MAX_USER_DISPLAY_LEN$}", d)),
            Box::new(|p: &Process| p.user_id().map(|u| u.clone())),
        ))
    }
}

/// Concatenate command arguments.
fn cmd_string(proc: &Process) -> OsString {
    proc.cmd()
        .iter()
        .map(|s| s.to_owned())
        .collect::<Vec<_>>()
        .join(&OsString::from(" "))
}

fn calculate_depth(pid: &Pid, tree_map: &BTreeMap<Pid, TreeNode>) -> usize {
    if let Some(node) = tree_map.get(&pid) {
        node.children
            .iter()
            .map(|(_, _, child_pid)| calculate_depth(child_pid, tree_map))
            .max()
            .unwrap_or(0)
            + 1
    } else {
        0
    }
}

fn print_tree(
    pid: &Pid,
    long: bool,
    tree_map: &BTreeMap<Pid, TreeNode>,
    processes: &HashMap<Pid, Process>,
    prefix: &str, // New argument for the tree branch prefix
    last: bool,   // Indicates if this is the last child
) -> Result<(), String> {
    let root = "root".to_string();
    let proc = processes.get(&pid).expect("process not found");
    let ppid = if let Some(ppid) = proc.parent() {
        format!("{}", ppid.as_u32())
    } else {
        root
    };

    let node = tree_map.get(&pid);
    let branch = if node.is_none() || node.unwrap().children.len() == 0 {
        if last {
            "└────"
        } else {
            "├────"
        }
    } else {
        // if last {
        //     "└───"
        // } else {
        //     "├───┐"
        // }
        if last {
            "└───┬"
        } else {
            "├───┬"
        }
    };

    my_println!(
        "{}{} {} ({}) {} {}",
        prefix,
        branch,
        pid.as_u32(),
        ppid,
        proc.name().to_string_lossy(),
        if long {
            cmd_string(proc)
        } else {
            OsString::default()
        }
        .to_string_lossy(),
    )?;

    // Print the children recursively.
    if let Some(node) = node {
        let child_count = node.children.len();
        for (i, (_, _, child_pid)) in node.children.iter().enumerate() {
            let is_last = i == child_count - 1;
            let new_prefix = if last {
                format!("{}    ", prefix)
            } else {
                format!("{}│   ", prefix)
            };

            print_tree(child_pid, long, tree_map, processes, &new_prefix, is_last)?;
        }
    }

    Ok(())
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
        flags.add_flag('l', "long", "Long format");
        flags.add_flag('t', "tree", "Display processes in a hierarchical view");
        flags.add_option('s', "sort", "Specify sorting order");

        Self { flags }
    }
}

impl Exec for ProcStatus {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();

        // Use forgiving, non-error checking parsing here, for compat with ps -efl, ps -afx etc.
        let _ = flags.parse_all(scope, args);

        if flags.is_present("help") {
            println!("Usage: ps [OPTIONS]");
            println!("List currently running processes and their details.");
            println!("\nOptions:");
            println!("{}", flags.help());
            println!("The sort spec is a comma-separated list of column names, optionally prefixed by a + or - sign.");
            println!("The PLUS sign specifies increasing sorting order (the default), and MINUS specifies decreasing order.");
            println!("Examples:\n\tps --sort name,-mem\n\tps -s \"+cpu,-mem,user\"\n");
            println!("\nNOTE: It is recommended to use the --long option in conjunction with the 'less' pager, e.g.: ps -al | less\n");
            return Ok(Value::success());
        }

        let tree_view = flags.is_present("tree");
        let long = flags.is_present("long");

        let mut view = View::new();

        view.columns.push(View::user_column());
        view.columns.push(View::pid_column());
        view.columns.push(View::parent_pid_column());
        view.columns.push(View::name_column());
        view.columns.push(View::cpu_usage_column());
        view.columns.push(View::mem_usage_column());
        view.columns.push(View::run_time_column());
        if long {
            view.columns.push(View::cmd_column());
        }

        if let Some(sort_spec) = flags.option("sort") {
            if tree_view {
                my_warning!(scope, "Sort ignored due to --tree option");
            }
            view.parse_sort_spec(scope, sort_spec, args)?;
        }

        if !flags.is_present("all") {
            view.filters.push(Box::new(UserProc::new(&view.system)));
        }

        if tree_view {
            view.process_tree(long)?;
        } else {
            view.process_list()?;
        }

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
