use super::{register_command, Exec, ShellCommand};
use crate::cmds::flags::CommandFlags;
use crate::eval::{Scope, Value};
use crate::my_println;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, BufRead};
use std::rc::Rc;

const MAX_CONTEXT: usize = 3;
struct Diff {
    flags: CommandFlags,
}

impl Diff {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        Diff { flags }
    }
}

impl Exec for Diff {
    fn exec(&self, name: &str, args: &Vec<String>, _: &Rc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse(args)?;

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... FILE1 FILE2", name);
            println!("Compare FILES line by line.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if filenames.len() != 2 {
            return Err("diff requires exactly two filenames".to_string());
        }

        let file1 = read_file(&filenames[0])?;
        let file2 = read_file(&filenames[1])?;

        let mut grid = Grid::new();
        diff(&file1, &file2, &mut grid);

        print_unified(&grid, &file1, &file2, &filenames[0], &filenames[1])?;

        Ok(Value::success())
    }

    fn is_external(&self) -> bool {
        false
    }
}

fn read_file(filename: &str) -> Result<Vec<String>, String> {
    let file = File::open(filename).map_err(|e| e.to_string())?;
    let reader = io::BufReader::new(file);
    reader
        .lines()
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

#[derive(Clone, Debug)]
enum Edit {
    None,
    Delete,
    Insert,
}

#[derive(Clone, Debug)]
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
        if n.i < dest.len() || n.j < src.len() {
            if let Some(m) = grid.at(n.i, n.j) {
                if m.d <= n.d {
                    continue;
                }
            }

            if n.i < dest.len() {
                if n.j < src.len() {
                    if &dest[n.i] == &src[n.j] {
                        queue.push_back(Node::new(n.i + 1, n.j + 1, n.d - 1, Edit::None));
                    } else {
                        queue.push_back(Node::new(n.i + 1, n.j, n.d - 1, Edit::Insert));
                        queue.push_back(Node::new(n.i, n.j + 1, n.d - 1, Edit::Delete));
                    }
                } else {
                    queue.push_back(Node::new(n.i + 1, n.j, n.d - 1, Edit::Insert));
                }
            } else if n.j < src.len() {
                queue.push_back(Node::new(n.i, n.j + 1, n.d - 1, Edit::Delete));
            }
        }
        grid.insert(n);
    }
}

/// Accumulate edit hunks for printing
struct Hunk<'a> {
    src: &'a [String],
    dest: &'a [String],
    src_path: &'a str,
    dest_path: &'a str,
    edits: Vec<String>,
    src_count: usize,
    src_line: usize,
    dest_count: usize,
    dest_line: usize,
    context: VecDeque<&'a str>,
    print_header: bool,
}

impl<'a> Hunk<'a> {
    fn new(src: &'a [String], dest: &'a [String], src_path: &'a str, dest_path: &'a str) -> Self {
        Self {
            src,
            dest,
            src_path,
            dest_path,
            edits: Vec::new(),
            src_count: 0,
            src_line: src.len(),
            dest_count: 0,
            dest_line: dest.len(),
            context: VecDeque::new(),
            print_header: true,
        }
    }

    fn update(&mut self, n: &Node) -> Result<bool, String> {
        match n.op {
            Edit::None => {
                self.src_line -= 1;
                self.dest_line -= 1;

                // Update the rolling buffer of context lines
                self.context.push_front(&self.src[self.src_line]);

                if self.context.len() == MAX_CONTEXT {
                    if !self.edits.is_empty() {
                        self.print()?;
                        self.reset();
                    }

                    self.context.pop_back();
                }
            }
            Edit::Delete => {
                self.src_count += 1;
                self.src_line -= 1;
                self.context.clear();
                self.edits.push(format!("-{}", &self.src[self.src_line]));
            }
            Edit::Insert => {
                self.dest_count += 1;
                self.dest_line -= 1;
                self.context.clear();
                self.edits.push(format!("+{}", &self.dest[self.dest_line]));
            }
        }

        Ok(self.src_line > 0 && self.dest_line > 0)
    }

    fn print(&self) -> Result<(), String> {
        if self.print_header {
            my_println!("--- {}", self.src_path)?;
            my_println!("+++ {}", self.dest_path)?;
        }

        let ctx_len = self.context.len();

        my_println!(
            "@@ -{},{} +{},{} @@",
            self.src_line + 1,
            self.src_count + ctx_len,
            self.dest_line + 1,
            self.dest_count + ctx_len
        )?;

        self.context
            .iter()
            .try_for_each(|line| my_println!(" {}", line))?;

        self.edits
            .iter()
            .rev()
            .try_for_each(|line| my_println!("{}", line))?;

        Ok(())
    }

    fn reset(&mut self) {
        self.context.clear();
        self.edits.clear();
        self.print_header = false;
        self.src_count = 0;
        self.dest_count = 0;
    }
}

fn print_unified(
    grid: &Grid,
    src: &[String],
    dest: &[String],
    src_path: &str,
    dest_path: &str,
) -> Result<(), String> {
    let mut hunk = Hunk::new(src, dest, src_path, dest_path);

    while let Some(edit) = grid.at(hunk.dest_line, hunk.src_line) {
        if !hunk.update(&edit)? {
            break;
        }
    }

    if !hunk.edits.is_empty() {
        hunk.print()?;
    }

    Ok(())
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "diff".to_string(),
        inner: Rc::new(Diff::new()),
    });
}
