use super::{register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::utils::format_error;
use crate::{cmds::flags::CommandFlags, eval::Value, scope::Scope};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    QueueableCommand,
};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use terminal_size::{terminal_size, Height, Width};

enum FileAction {
    NextFile,
    PrevFile,
    Quit,
}

struct LessViewer {
    lines: Vec<String>,
    current_line: usize,
    horizontal_scroll: usize,
    screen_width: usize,
    screen_height: usize,
    last_search: Option<String>,
    last_search_direction: bool,
    line_num_width: usize,
    search_start_index: usize,
    show_line_numbers: bool,
    status: Option<String>,
    use_color: bool,
}

impl LessViewer {
    fn new<R: BufRead>(reader: R) -> io::Result<Self> {
        let lines: Vec<String> = reader.lines().collect::<io::Result<_>>()?;
        let (Width(w), Height(h)) = terminal_size().unwrap_or((Width(80), Height(24)));

        Ok(LessViewer {
            line_num_width: lines.len().to_string().len() + 1,
            lines,
            current_line: 0,
            horizontal_scroll: 0,
            screen_width: w as usize,
            screen_height: h.saturating_sub(1) as usize,
            last_search: None,
            last_search_direction: true,
            search_start_index: 0,
            show_line_numbers: false,
            status: None,
            use_color: true,
        })
    }

    fn clear_search(&mut self) {
        self.last_search = None;
    }

    fn display_page<W: Write>(&self, stdout: &mut W, buffer: &mut String) -> io::Result<()> {
        buffer.clear();
        buffer.push_str("\r\n");

        let end = (self.current_line + self.screen_height).min(self.lines.len());

        for (index, line) in self.lines[self.current_line..end].iter().enumerate() {
            if self.show_line_numbers {
                let line_number = self.current_line + index + 1;
                buffer.push_str(&format!("{:>w$}", line_number, w = self.line_num_width));
            }
            self.display_line(line, buffer)?;
        }

        // Fill any remaining lines with empty space
        for _ in end..self.current_line + self.screen_height {
            if self.show_line_numbers {
                buffer.push_str(&" ".repeat(self.screen_width.saturating_sub(1)));
            }
            buffer.push_str("\r\n");
        }

        if let Some(ref message) = self.status {
            buffer.push_str(message);
        } else {
            buffer.push(':');
        }

        write!(stdout, "{}", buffer)?;
        stdout.flush()?;

        Ok(())
    }

    fn display_line(&self, line: &str, buffer: &mut String) -> io::Result<()> {
        // Determine the effective width of the line to be displayed
        let effective_width = if self.show_line_numbers {
            self.screen_width.saturating_sub(self.line_num_width + 2)
        } else {
            self.screen_width
        };

        // Compute the starting point based on horizontal scroll
        let start_index = self.horizontal_scroll.min(line.len());
        let end_index = (start_index + effective_width).min(line.len());

        if self.show_line_numbers {
            buffer.push_str("  ");
        }

        // Handle search highlighting if present
        if let Some(ref search) = self.last_search {
            let mut start = start_index;
            while let Some(index) = line[start..end_index].find(search) {
                let search_start = start + index;
                let search_end = search_start + search.len();

                // Add text before the search match
                buffer.push_str(&line[start..search_start]);

                // Highlight the search term if colors are enabled
                if self.use_color {
                    buffer.push_str("\x1b[43m\x1b[30m");
                }
                buffer.push_str(&line[search_start..search_end]);

                // Reset color after the match
                if self.use_color {
                    buffer.push_str("\x1b[0m");
                }

                // Move start after the matched search term
                start = search_end;
            }

            // Append any remaining text after the last search match
            buffer.push_str(&line[start..end_index]);
        } else {
            // If no search, append the entire visible portion of the line
            buffer.push_str(&line[start_index..end_index]);
        }

        buffer.push_str("\r\n");

        Ok(())
    }

    fn last_page(&mut self) {
        if self.lines.is_empty() {
            self.current_line = 0;
        } else {
            self.current_line = self.lines.len().saturating_sub(self.screen_height);
        }
    }

    fn next_line(&mut self) {
        if self.current_line < self.lines.len().saturating_sub(1) {
            self.current_line += 1;
            if self.current_line + self.screen_height > self.lines.len() {
                self.current_line = self.lines.len().saturating_sub(self.screen_height);
            }
        }
    }

    fn next_page(&mut self) {
        let new_line =
            (self.current_line + self.screen_height).min(self.lines.len().saturating_sub(1));
        if new_line > self.current_line {
            self.current_line = new_line;
            if self.current_line + self.screen_height > self.lines.len() {
                self.current_line = self.lines.len().saturating_sub(self.screen_height);
            }
        }
    }

    fn prev_page(&mut self) {
        self.current_line = self.current_line.saturating_sub(self.screen_height);
    }

    fn prev_line(&mut self) {
        if self.current_line > 0 {
            self.current_line -= 1;
        }
    }

    fn scroll_right(&mut self) {
        self.horizontal_scroll += 1;
    }

    fn scroll_left(&mut self) {
        self.horizontal_scroll = self.horizontal_scroll.saturating_sub(1);
    }

    fn search(&mut self, query: &str, forward: bool) {
        self.last_search = Some(query.to_string());
        self.last_search_direction = forward;

        let mut found = false;

        if forward {
            for (index, line) in self.lines[self.search_start_index..].iter().enumerate() {
                if line.contains(query) {
                    self.current_line = self.search_start_index + index;
                    self.search_start_index = self.current_line + 1;
                    found = true;
                    break;
                }
            }
        } else {
            for (index, line) in self.lines[..self.search_start_index]
                .iter()
                .rev()
                .enumerate()
            {
                if line.contains(query) {
                    self.current_line = self.search_start_index - index - 1;
                    self.search_start_index = self.current_line;
                    found = true;
                    break;
                }
            }
        }

        if !found {
            self.status = Some(format!("Pattern not found: {}", query));
            self.search_start_index = if forward {
                self.current_line + 1
            } else {
                self.current_line
            };
        }

        if self.current_line + self.screen_height > self.lines.len() {
            self.current_line = self.lines.len().saturating_sub(self.screen_height);
        }
    }

    fn repeat_search(&mut self) {
        if let Some(query) = self.last_search.clone() {
            let direction = self.last_search_direction;
            self.search(&query, direction);
        }
    }

    fn run(&mut self) -> io::Result<FileAction> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        let mut action = FileAction::NextFile;
        let mut buffer = String::with_capacity(self.screen_width * self.screen_height);

        self.display_page(&mut stdout, &mut buffer)?;

        loop {
            let (mut current_line, horizontal_scroll, search_dir, search_term, show_lines) = (
                self.current_line,
                self.horizontal_scroll,
                self.last_search_direction,
                self.last_search.clone(),
                self.show_line_numbers,
            );
            self.status = None;

            if let Event::Key(key_event) = event::read()? {
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
                        }
                        break;
                    }
                    KeyCode::Char('q') => {
                        action = FileAction::Quit;
                        break;
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
                            cursor::SavePosition,
                            cursor::MoveTo(0, self.screen_height as u16),
                            Clear(ClearType::CurrentLine),
                            cursor::RestorePosition
                        )?;

                        let (prompt, forward) = if key_event.code == KeyCode::Char('/') {
                            ("Search forward: ", true)
                        } else {
                            self.search_start_index = self.lines.len().saturating_sub(1);
                            ("Search backward: ", false)
                        };

                        let query = self.prompt_for_command(&prompt)?;
                        if query.is_empty() {
                            self.status = None;
                            current_line = usize::MAX;
                        } else {
                            self.search(&query, forward);
                        }
                    }
                    KeyCode::Char('n') => {
                        self.repeat_search();
                    }
                    KeyCode::Char('l') => {
                        self.show_line_numbers = !self.show_line_numbers;
                    }
                    _ => {}
                }

                if current_line != self.current_line
                    || horizontal_scroll != self.horizontal_scroll
                    || search_dir != self.last_search_direction
                    || search_term != self.last_search
                    || show_lines != self.show_line_numbers
                {
                    self.display_page(&mut stdout, &mut buffer)?;
                }
            }
        }

        execute!(stdout, LeaveAlternateScreen)?;
        disable_raw_mode()?;

        Ok(action)
    }

    fn prompt_for_command(&mut self, prompt: &str) -> io::Result<String> {
        let mut stdout = io::stdout();
        stdout
            .queue(cursor::SavePosition)?
            .queue(cursor::MoveTo(0, self.screen_height as u16))?
            .queue(Clear(ClearType::CurrentLine))?
            .flush()?;

        let cmd = crate::prompt::read_input(prompt)?;

        stdout.queue(cursor::RestorePosition)?.flush()?;
        Ok(cmd.trim().to_string())
    }

    fn show_help(&self) -> io::Result<()> {
        let help_text = if self.use_color {
            "\x1b[7mb\x1b[0m Prev Page | \
            \x1b[7mf\x1b[0m Next Page | \
            \x1b[7m/\x1b[0m Search | \
            \x1b[7m?\x1b[0m Search Backward | \
            \x1b[7m:n\x1b[0m Next File | \
            \x1b[7m:p\x1b[0m Prev File | \
            \x1b[7m:q\x1b[0m Quit"
        } else {
            "b Prev Page | f Next Page | / Search | ? Search Backward | :n Next File | :p Prev File | :q Quit"
        };

        io::stdout()
            .queue(cursor::SavePosition)?
            .queue(cursor::MoveTo(0, self.screen_height as u16))?
            .queue(Print(help_text))?
            .flush()
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
            run_less_viewer(scope, &flags, reader, None).map_err(|e| e.to_string())?;
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

                match run_less_viewer(scope, &flags, reader, Some(filename.clone()))
                    .map_err(|e| e.to_string())?
                {
                    FileAction::PrevFile => i = i.saturating_sub(1),
                    FileAction::NextFile => i = std::cmp::min(i + 1, filenames.len() - 1),
                    FileAction::Quit => break,
                }
            }
        };

        Ok(Value::success())
    }
}

fn run_less_viewer<R: BufRead>(
    scope: &Arc<Scope>,
    flags: &CommandFlags,
    reader: R,
    filename: Option<String>,
) -> io::Result<FileAction> {
    let mut viewer = LessViewer::new(reader)?;

    viewer.show_line_numbers = flags.is_present("number");
    viewer.use_color = scope.use_colors(&std::io::stdout());
    viewer.status = filename;

    viewer.run()
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "less".to_string(),
        inner: Rc::new(Less::new()),
    });
}
