use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::{eval::Value, scope::Scope, symlnk::SymLink, utils::format_error};
use colored::*;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead};
use std::path::Path;
use std::rc::Rc;

struct Diff {
    flags: CommandFlags,
}

impl Diff {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('o', "color", "Color output");

        Self { flags }
    }
}

impl Exec for Diff {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let fnames = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... FILE1 FILE2", name);
            println!("Compare FILES line by line.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if fnames.len() != 2 {
            return Err("diff requires exactly two filenames".to_string());
        }

        let mut files = Vec::new();

        for filename in fnames.iter().take(2) {
            let path = Path::new(filename)
                .resolve()
                .map_err(|e| format_error(scope, filename, args, e))?;

            files.push(read_file(filename, &path, scope, args)?);
        }

        // Calculate the diff
        let mut grid = Grid::new();
        diff(&files[0], &files[1], &mut grid);

        let color = flags.is_present("color") && scope.use_colors(&std::io::stdout());

        // unified view with no context lines.
        print(&grid, &files[0], &files[1], &fnames[0], &fnames[1], color)?;

        Ok(Value::success())
    }
}

fn read_file(
    filename: &str, // As given in the command line
    path: &Path,    // Resolved path
    scope: &Rc<Scope>,
    args: &Vec<String>,
) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| format_error(scope, filename, args, e))?;

    io::BufReader::new(file)
        .lines()
        .collect::<Result<_, _>>()
        .map_err(|e| format_error(scope, filename, args, e))
}

#[derive(Clone)]
enum Edit {
    None,
    Delete,
    Insert,
}

#[derive(Clone)]
struct Node {
    i: usize,
    j: usize,
    d: usize,
    op: Edit,
}

impl Node {
    fn new(i: usize, j: usize, d: usize, op: Edit) -> Self {
        Self { i, j, d, op }
    }
}

struct Grid {
    nodes: Vec<Vec<Option<Node>>>,
}

impl Grid {
    fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    fn at(&self, i: usize, j: usize) -> Option<Node> {
        if self.nodes.len() <= i {
            None
        } else if self.nodes[i].len() <= j {
            None
        } else {
            self.nodes[i][j].clone()
        }
    }

    fn insert(&mut self, n: Node) {
        let (i, j) = (n.i, n.j);
        if self.nodes.len() <= i {
            self.nodes.resize(n.i + 1, Vec::new());
        }
        if self.nodes[i].len() <= j {
            self.nodes[i].resize(j + 1, None);
        }
        self.nodes[i][j] = Some(n);
    }
}

fn diff(src: &[String], dest: &[String], grid: &mut Grid) {
    let mut queue = VecDeque::new();

    queue.push_back(Node::new(0, 0, src.len() + dest.len(), Edit::None));

    while let Some(n) = queue.pop_front() {
        if let Some(m) = grid.at(n.i, n.j) {
            if m.d <= n.d {
                continue;
            }
        }

        if n.i < dest.len() {
            if n.j < src.len() {
                if &dest[n.i] == &src[n.j] {
                    queue.push_back(Node::new(n.i + 1, n.j + 1, n.d - 2, Edit::None));
                } else {
                    queue.push_back(Node::new(n.i, n.j + 1, n.d - 1, Edit::Delete));
                    queue.push_back(Node::new(n.i + 1, n.j, n.d - 1, Edit::Insert));
                }
            } else {
                queue.push_back(Node::new(n.i + 1, n.j, n.d - 1, Edit::Insert));
            }
        } else if n.j < src.len() {
            queue.push_back(Node::new(n.i, n.j + 1, n.d - 1, Edit::Delete));
        }
        grid.insert(n);
    }
}

struct Hunk {
    edits: Vec<String>,
    src_count: usize,
    src_line: usize,
    dest_count: usize,
    dest_line: usize,
}

impl Hunk {
    fn new() -> Self {
        Self {
            edits: Vec::new(),
            src_count: 0,
            src_line: 0,
            dest_count: 0,
            dest_line: 0,
        }
    }

    fn update(&mut self, src_line: usize, dest_line: usize) -> bool {
        self.dest_line = dest_line;
        if self.dest_count == 0 && dest_line > 0 {
            self.dest_line -= 1;
        }

        self.src_line = src_line;
        if self.src_count == 0 && src_line > 0 {
            self.src_line -= 1;
        }

        !self.edits.is_empty()
    }
}

/// Accumulates edit hunks for printing
struct UnifiedView<'a> {
    src: &'a [String],
    dest: &'a [String],
    src_line: usize,  // Current line number in the 'src' file
    dest_line: usize, // Current line number in the 'dest' file
    hunks: Vec<Hunk>,
}

impl<'a> UnifiedView<'a> {
    fn new(src: &'a [String], dest: &'a [String]) -> Self {
        Self {
            src,
            dest,
            src_line: src.len(),
            dest_line: dest.len(),
            hunks: vec![Hunk::new()],
        }
    }

    fn hunk(&mut self) -> &mut Hunk {
        self.hunks.last_mut().unwrap()
    }

    fn update(&mut self, n: &Node) -> bool {
        match n.op {
            Edit::None => {
                self.push_hunk(false);
                self.src_line -= 1;
                self.dest_line -= 1;
            }
            Edit::Delete => {
                self.src_line -= 1;
                let line = &self.src[self.src_line];
                self.hunk().src_count += 1;
                self.hunk().edits.push(format!("-{}", line));
            }
            Edit::Insert => {
                self.dest_line -= 1;
                let line = &self.dest[self.dest_line];
                self.hunk().dest_count += 1;
                self.hunk().edits.push(format!("+{}", line));
            }
        }

        self.src_line != 0 || self.dest_line != 0
    }

    fn print(&mut self, src_path: &str, dest_path: &str, color: bool) -> Result<(), String> {
        if self.hunks.len() > 1 {
            my_println!("--- {}", src_path.replace("\\", "/"))?;
            my_println!("+++ {}", dest_path.replace("\\", "/"))?;
        }

        for hunk in self.hunks.iter().rev() {
            if hunk.edits.is_empty() {
                continue;
            }
            my_println!(
                "@@ -{},{} +{},{} @@",
                hunk.src_line + 1,
                hunk.src_count,
                hunk.dest_line + 1,
                hunk.dest_count
            )?;

            hunk.edits.iter().rev().try_for_each(|line| {
                let output_line = if color && line.starts_with("-") {
                    line.red()
                } else if color && line.starts_with("+") {
                    line.green()
                } else {
                    line.normal()
                };
                my_println!("{}", output_line)
            })?;
        }
        Ok(())
    }

    fn push_hunk(&mut self, last: bool) {
        let (src_line, dest_line) = (self.src_line, self.dest_line);

        if self.hunk().update(src_line, dest_line) && !last {
            self.hunks.push(Hunk::new());
        }
    }
}

fn print(
    grid: &Grid,
    src: &[String],
    dest: &[String],
    src_path: &str,
    dest_path: &str,
    color: bool,
) -> Result<(), String> {
    let mut unified = UnifiedView::new(src, dest);

    while let Some(edit) = grid.at(unified.dest_line, unified.src_line) {
        if !unified.update(&edit) {
            break;
        }
    }
    unified.push_hunk(true);
    unified.print(src_path, dest_path, color)
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "diff".to_string(),
        inner: Rc::new(Diff::new()),
    });
}
