use super::{register_command, Exec, ShellCommand};
use crate::prompt;
use crate::{
    cmds::flags::CommandFlags, eval::Value, scope::Scope, symlnk::SymLink, utils::format_error,
};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::Print,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    QueueableCommand,
};
use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use terminal_size::{terminal_size, Height, Width};

enum FileAction {
    None,
    NextFile,
    PrevFile,
    Quit,
}

#[derive(Clone, Debug, PartialEq)]
struct ViewerState {
    current_line: usize,
    horizontal_scroll: usize,
    last_search: Option<String>,
    last_search_direction: bool,
    redraw: bool, // Force redraw
    search_start_index: usize,
    show_line_numbers: bool,
    status_line: Option<String>,
}

impl ViewerState {
    fn new() -> Self {
        Self {
            current_line: 0,
            horizontal_scroll: 0,
            redraw: false,
            last_search: None,
            last_search_direction: true,
            search_start_index: 0,
            show_line_numbers: false,
            status_line: None,
        }
    }
}

struct Viewer {
    lines: Vec<String>,
    line_num_width: usize,
    screen_width: usize,
    screen_height: usize,
    state: ViewerState,
    use_color: bool,
}

impl Viewer {
    fn new<R: BufRead>(reader: R) -> io::Result<Self> {
        let lines: Vec<String> = reader.lines().collect::<io::Result<_>>()?;

        let (Width(w), Height(h)) = terminal_size().unwrap_or((Width(80), Height(24)));

        Ok(Self {
            line_num_width: lines.len().to_string().len() + 1,
            lines,

            screen_width: w as usize,
            screen_height: h.saturating_sub(1) as usize,
            state: ViewerState::new(),
            use_color: true,
        })
    }

    fn clear_search(&mut self) {
        self.state.last_search = None;
    }

    fn display_page<W: Write>(&self, stdout: &mut W, buffer: &mut String) -> io::Result<()> {
        buffer.clear();

        let end = (self.state.current_line + self.screen_height).min(self.lines.len());

        for (index, line) in self.lines[self.state.current_line..end].iter().enumerate() {
            if self.state.show_line_numbers {
                let line_number = self.state.current_line + index + 1;
                buffer.push_str(&format!("{:>w$}", line_number, w = self.line_num_width));
            }
            self.display_line(line, buffer)?;
        }

        // Fill any remaining lines
        for _ in end..self.state.current_line + self.screen_height {
            buffer.push('~');
            buffer.push_str(&" ".repeat(self.screen_width.saturating_sub(1)));
            buffer.push_str("\r\n");
        }

        execute!(
            stdout,
            cursor::Hide,
            cursor::MoveTo(0, 0),
            Print(buffer),
            cursor::MoveTo(0, self.screen_height as u16),
            Clear(ClearType::CurrentLine),
            cursor::Show,
        )?;

        // Update the "status / hints" line
        if let Some(ref message) = self.state.status_line {
            write!(stdout, "{}", message)?;
        } else {
            write!(stdout, ":")?;
        }
        stdout.flush()?;

        Ok(())
    }

    fn display_line(&self, line: &str, buffer: &mut String) -> io::Result<()> {
        // Determine the effective width of the line to be displayed
        let effective_width = if self.state.show_line_numbers {
            self.screen_width.saturating_sub(self.line_num_width + 2)
        } else {
            self.screen_width
        };

        // Compute the starting point based on horizontal scroll
        let start_index = self.state.horizontal_scroll.min(line.len());
        let end_index = (start_index + effective_width).min(line.len());

        if self.state.show_line_numbers {
            buffer.push_str("  ");
        }

        // Handle search highlighting if present
        if let Some(ref search) = self.state.last_search {
            let mut start = start_index;
            while let Some(index) = line[start..end_index].find(search) {
                let search_start = start + index;
                let search_end = search_start + search.len();

                // Add text before the search match
                buffer.push_str(&line[start..search_start]);

                // Highlight the search term if colors are enabled
                buffer.push_str(&self.strong(&line[search_start..search_end]));

                // Move start after the matched search term
                start = search_end;
            }

            // Append any remaining text after the last search match
            buffer.push_str(&line[start..end_index]);
        } else {
            // If no search, append the entire visible portion of the line
            buffer.push_str(&line[start_index..end_index]);
        }

        // Clear to the end of the line
        buffer.push_str(&" ".repeat(effective_width.saturating_sub(end_index - start_index)));
        buffer.push_str("\r\n");

        Ok(())
    }

    fn goto_line(&mut self, cmd: &str) {
        self.state.status_line = None;
        let num_str = cmd.trim();

        if let Ok(number) = num_str.parse::<usize>() {
            if number < 1 || number > self.lines.len() {
                self.state.status_line = Some(
                    self.strong(&format!(
                        "{} is out of range: [1-{}]",
                        number,
                        self.lines.len()
                    ))
                    .into(),
                );
            } else {
                self.state.current_line = number.saturating_sub(1);
            }
        } else {
            self.state.status_line = Some(self.strong("Invalid line number").to_string());
        }
    }

    fn last_page(&mut self) {
        if self.lines.is_empty() {
            self.state.current_line = 0;
        } else {
            self.state.current_line = self.lines.len().saturating_sub(self.screen_height);
        }
    }

    fn next_line(&mut self) {
        if self.state.current_line < self.lines.len().saturating_sub(1) {
            self.state.current_line += 1;
            if self.state.current_line + self.screen_height > self.lines.len() {
                self.state.current_line = self.lines.len().saturating_sub(self.screen_height);
            }
        }
    }

    fn next_page(&mut self) {
        let new_line =
            (self.state.current_line + self.screen_height).min(self.lines.len().saturating_sub(1));
        if new_line > self.state.current_line {
            self.state.current_line = new_line;
            if self.state.current_line + self.screen_height > self.lines.len() {
                self.state.current_line = self.lines.len().saturating_sub(self.screen_height);
            }
        }
    }

    fn prev_page(&mut self) {
        self.state.current_line = self.state.current_line.saturating_sub(self.screen_height);
    }

    fn prev_line(&mut self) {
        if self.state.current_line > 0 {
            self.state.current_line -= 1;
        }
    }

    fn scroll_right(&mut self) {
        self.state.horizontal_scroll += 1;
    }

    fn scroll_left(&mut self) {
        self.state.horizontal_scroll = self.state.horizontal_scroll.saturating_sub(1);
    }

    fn search(&mut self, query: &str, forward: bool) {
        self.state.last_search = Some(query.to_string());
        self.state.last_search_direction = forward;

        let mut found = false;
        let search_start_index = self.state.search_start_index;
        if forward {
            for (index, line) in self.lines[self.state.search_start_index..]
                .iter()
                .enumerate()
            {
                if line.contains(query) {
                    self.state.current_line = self.state.search_start_index + index;
                    self.state.search_start_index = self.state.current_line + 1;
                    found = true;
                    break;
                }
            }
        } else {
            for (index, line) in self.lines[..self.state.search_start_index]
                .iter()
                .rev()
                .enumerate()
            {
                if line.contains(query) {
                    self.state.current_line = self.state.search_start_index - index - 1;
                    self.state.search_start_index = self.state.current_line;
                    found = true;
                    break;
                }
            }
        }

        if found {
            self.state.status_line = None;
        } else {
            self.state.status_line =
                Some(self.strong(&format!("Pattern not found: {}", query)).into());
            self.state.search_start_index = search_start_index;
        }
    }

    fn repeat_search(&mut self) {
        if let Some(query) = self.state.last_search.clone() {
            let direction = self.state.last_search_direction;
            self.search(&query, direction);
        }
    }

    fn run(&mut self) -> io::Result<FileAction> {
        let _raw_mode = prompt::RawMode::new()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, cursor::MoveTo(0, 0),)?;

        let mut action = FileAction::None;
        let mut buffer = String::with_capacity(self.screen_width * self.screen_height);

        self.display_page(&mut stdout, &mut buffer)?;

        // Process events
        while matches!(action, FileAction::None) {
            let mut state = self.state.clone();

            let event = event::read()?;

            if let Event::Resize(w, h) = event {
                self.screen_width = w.into();
                self.screen_height = h.saturating_sub(1).into();
                state.redraw = true;
            }

            if let Event::Key(key_event) = event {
                if key_event.kind != KeyEventKind::Press {
                    continue;
                }

                match key_event.code {
                    KeyCode::F(1) => self.show_help()?,
                    KeyCode::Char('h') => self.show_help()?,
                    KeyCode::Char(':') => {
                        let cmd = self.prompt_for_command(":")?;
                        if cmd == "n" {
                            action = FileAction::NextFile;
                        } else if cmd == "p" {
                            action = FileAction::PrevFile;
                        } else if cmd == "q" {
                            action = FileAction::Quit;
                        } else {
                            self.goto_line(&cmd);
                        }
                    }
                    KeyCode::Char('q') => {
                        action = FileAction::Quit;
                    }
                    KeyCode::Char('b') => self.prev_page(),
                    KeyCode::Char('f') => self.next_page(),
                    KeyCode::Char(' ') => self.next_page(),
                    KeyCode::Char('G') => self.last_page(),
                    KeyCode::Esc => self.clear_search(),
                    KeyCode::Enter => self.next_line(),
                    KeyCode::Up => self.prev_line(),
                    KeyCode::Down => self.next_line(),
                    KeyCode::Left => self.scroll_left(),
                    KeyCode::Right => self.scroll_right(),
                    KeyCode::PageUp => self.prev_page(),
                    KeyCode::PageDown => self.next_page(),
                    KeyCode::Char('/') | KeyCode::Char('?') => {
                        execute!(
                            stdout,
                            cursor::MoveTo(0, self.screen_height as u16),
                            Clear(ClearType::CurrentLine),
                        )?;

                        let (prompt, forward) = if key_event.code == KeyCode::Char('/') {
                            ("Search forward: ", true)
                        } else {
                            self.state.search_start_index = self.lines.len().saturating_sub(1);
                            ("Search backward: ", false)
                        };

                        let query = self.prompt_for_command(&prompt)?;
                        if query.is_empty() {
                            self.state.status_line = None;
                            state.redraw = true;
                        } else {
                            self.search(&query, forward);
                        }
                    }
                    KeyCode::Char('n') => {
                        self.repeat_search();
                    }
                    KeyCode::Char('l') => {
                        self.state.show_line_numbers = !self.state.show_line_numbers;
                    }
                    _ => {}
                }
            }
            if self.state != state {
                self.display_page(&mut stdout, &mut buffer)?;
            }
        }
        execute!(stdout, LeaveAlternateScreen)?;
        Ok(action)
    }

    fn prompt_for_command(&mut self, prompt: &str) -> io::Result<String> {
        let mut stdout = io::stdout();
        stdout
            .queue(cursor::MoveTo(0, self.screen_height as u16))?
            .queue(Clear(ClearType::CurrentLine))?
            .flush()?;

        let cmd = crate::prompt::read_input(prompt)?;

        Ok(cmd.trim().to_string())
    }

    /// Show temporary hints on the last ("status") line.
    fn show_help(&self) -> io::Result<()> {
        let help_items = vec![
            ("b", "Prev Page"),
            ("f", "Next Page"),
            ("/", "Search"),
            ("?", "Search Backward"),
            (":n", "Next File"),
            (":p", "Prev File"),
            (":q", "Quit"),
        ];

        let help_text = help_items
            .iter()
            .map(|(key, description)| format!("{} {}", self.strong(key), description))
            .collect::<Vec<String>>()
            .join(" | ");

        io::stdout()
            .queue(cursor::MoveTo(0, self.screen_height as u16))?
            .queue(Print(&help_text))?
            .flush()
    }

    fn strong<'a>(&self, s: &'a str) -> Cow<'a, str> {
        if self.use_color {
            Cow::Owned(format!("\x1b[7m{}\x1b[0m", s))
        } else {
            Cow::Borrowed(s)
        }
    }
}

struct Less {
    flags: CommandFlags,
}

impl Less {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('?', "help", "Display this help message");
        flags.add_flag('n', "number", "Number output lines");
        Less { flags }
    }
}

impl Exec for Less {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... [FILE]...", name);
            println!("View FILE(s) or standard input in a pager.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        if filenames.is_empty() {
            let stdin = io::stdin();
            let reader = stdin.lock();
            run_viewer(scope, &flags, reader, None).map_err(|e| e.to_string())?;
        } else {
            let mut i: usize = 0;
            loop {
                let filename = filenames.get(i).unwrap();
                let path = Path::new(filename)
                    .resolve()
                    .map_err(|e| format_error(&scope, filename, args, e))?;

                let file =
                    File::open(&path).map_err(|e| format_error(&scope, filename, args, e))?;
                let reader = BufReader::new(file);

                match run_viewer(
                    scope,
                    &flags,
                    reader,
                    Some(format!("{} ({} of {})", filename, i + 1, filenames.len())),
                )
                .map_err(|e| e.to_string())?
                {
                    FileAction::PrevFile => i = i.saturating_sub(1),
                    FileAction::NextFile => i = std::cmp::min(i + 1, filenames.len() - 1),
                    FileAction::Quit => break,
                    FileAction::None => {}
                }
            }
        };

        Ok(Value::success())
    }
}

fn run_viewer<R: BufRead>(
    scope: &Arc<Scope>,
    flags: &CommandFlags,
    reader: R,
    filename: Option<String>,
) -> io::Result<FileAction> {
    let mut viewer = Viewer::new(reader)?;

    viewer.state.show_line_numbers = flags.is_present("number");
    viewer.use_color = scope.use_colors(&std::io::stdout());
    if let Some(filename) = &filename {
        viewer.state.status_line = Some(viewer.strong(&filename).into());
    }

    viewer.run()
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "less".to_string(),
        inner: Rc::new(Less::new()),
    });
}
