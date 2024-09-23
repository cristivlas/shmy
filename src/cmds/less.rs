use super::{register_command, Exec, ShellCommand};
use crate::{
    cmds::flags::CommandFlags, eval::Value, prompt, scope::Scope, symlnk::SymLink,
    utils::format_error,
};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::Print,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
    QueueableCommand,
};
use memmap2::Mmap;
use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;

enum FileAction {
    None,
    NextFile,
    PrevFile,
    Quit,
}

// Constant threshold for switching between strategies.
// TODO: dynamically adapt based on available memory.
const MEMORY_MAPPED_THRESHOLD: u64 = 10 * 1024 * 1024;

// Abstraction for file content
trait FileContent {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<Cow<'_, str>>;
}

// In-memory strategy
struct InMemoryContent {
    lines: Vec<String>,
}

impl InMemoryContent {
    fn new<R: BufRead>(reader: R) -> io::Result<Self> {
        let lines: Vec<String> = reader.lines().collect::<io::Result<_>>()?;
        Ok(Self { lines })
    }
}

impl FileContent for InMemoryContent {
    fn len(&self) -> usize {
        self.lines.len()
    }

    fn get(&self, index: usize) -> Option<Cow<'_, str>> {
        self.lines
            .get(index)
            .and_then(|s| Some(Cow::Borrowed(s.as_str())))
    }
}

// Memory-mapped strategy
struct MemoryMappedContent {
    mmap: Mmap,
    line_offsets: Vec<usize>,
}

impl MemoryMappedContent {
    fn new(file: &File) -> io::Result<Self> {
        let mmap = unsafe { Mmap::map(file)? };
        let line_offsets = Self::calculate_line_offsets(&mmap);
        Ok(Self { mmap, line_offsets })
    }

    fn calculate_line_offsets(mmap: &Mmap) -> Vec<usize> {
        let mut offsets = vec![0];
        let mut iter = mmap.iter().enumerate();

        while let Some((i, &byte)) = iter.next() {
            if byte == b'\n' {
                offsets.push(i + 1);
            } else if byte >= 0x80 {
                // Start of a multi-byte UTF-8 character
                let mut char_bytes = vec![byte];

                // Collect all bytes of the multi-byte character
                while let Some((_, &b)) = iter.next() {
                    char_bytes.push(b);
                    if b < 0x80 || b >= 0xC0 {
                        break;
                    }
                }

                // Check if it's a valid UTF-8 character
                if std::str::from_utf8(&char_bytes).is_err() {
                    // Invalid UTF-8 sequence, treat each byte as a separate character
                    for _ in 1..char_bytes.len() {
                        if let Some((j, _)) = iter.next() {
                            if mmap[j] == b'\n' {
                                offsets.push(j + 1);
                            }
                        }
                    }
                }
            }
        }
        offsets
    }
}

impl FileContent for MemoryMappedContent {
    fn len(&self) -> usize {
        self.line_offsets.len()
    }

    fn get(&self, index: usize) -> Option<Cow<'_, str>> {
        if index >= self.line_offsets.len() {
            return None;
        }
        let start = self.line_offsets[index];
        let end = self
            .line_offsets
            .get(index + 1)
            .copied()
            .unwrap_or(self.mmap.len());

        Some(String::from_utf8_lossy(&self.mmap[start..end]))
    }
}

// Factory function to create the appropriate FileContent instance
fn create_file_content(
    scope: &Arc<Scope>,
    path: Option<&Path>,
) -> io::Result<Box<dyn FileContent>> {
    if let Some(path) = path {
        let file = File::open(path)?;
        let metadata = file.metadata()?;

        if metadata.len() > MEMORY_MAPPED_THRESHOLD {
            Ok(Box::new(MemoryMappedContent::new(&file)?))
        } else {
            let reader = BufReader::new(file);
            Ok(Box::new(InMemoryContent::new(reader)?))
        }
    } else {
        scope.show_eof_hint();
        Ok(Box::new(InMemoryContent::new(io::stdin().lock())?))
    }
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
    file_info: Option<String>,
    lines: Box<dyn FileContent>,
    line_num_width: usize,
    screen_width: usize,
    screen_height: usize,
    state: ViewerState,
}

impl Viewer {
    fn new(scope: &Arc<Scope>, file_info: Option<String>, path: Option<&Path>) -> io::Result<Self> {
        let content = create_file_content(scope, path)?;
        let line_num_width = content.len().to_string().len() + 1;

        let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));

        Ok(Self {
            file_info,
            lines: content,
            line_num_width,
            screen_width: w as usize,
            screen_height: h.saturating_sub(1) as usize,
            state: ViewerState::new(),
        })
    }

    fn clear_search(&mut self) {
        self.state.last_search = None;
    }

    fn display_page<W: Write>(&mut self, stdout: &mut W, buffer: &mut String) -> io::Result<()> {
        buffer.clear();

        let end = (self.state.current_line + self.screen_height).min(self.lines.len());

        for index in self.state.current_line..end {
            buffer.push_str("\x1b[2K"); // Clear line

            if self.state.show_line_numbers {
                let line_number = index + 1;
                buffer.push_str(&format!("{:>w$}  ", line_number, w = self.line_num_width));
            }

            if let Some(line) = self.lines.get(index) {
                self.display_line(&line.trim_end(), buffer)?;
            }
        }

        // Fill any remaining empty lines
        for _ in end..self.state.current_line + self.screen_height {
            buffer.push_str("\x1b[2K~\r\n");
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

        self.state.status_line = self
            .file_info
            .as_ref()
            .and_then(|info| Some(self.strong(&info)));

        Ok(())
    }

    fn display_line(&self, line: &str, buffer: &mut String) -> io::Result<()> {
        fn adjust_index_to_utf8_boundary(line: &str, index: usize) -> usize {
            if index >= line.len() {
                return line.len();
            }
            // Find the nearest valid UTF-8 boundary
            line.char_indices()
                .take_while(|&(i, _)| i <= index)
                .last()
                .map_or(0, |(i, _)| i)
        }

        // Determine the effective width of the line to be displayed
        let effective_width = if self.state.show_line_numbers {
            self.screen_width.saturating_sub(self.line_num_width + 2)
        } else {
            self.screen_width
        };

        // Compute the starting point based on horizontal scroll
        let start_index = self.state.horizontal_scroll.min(line.len());
        let end_index = (start_index + effective_width).min(line.len());

        // Adjust at UTF8 boundary so we don't panic when taking a slice of the line.
        let start_index = adjust_index_to_utf8_boundary(line, start_index);
        let end_index = adjust_index_to_utf8_boundary(line, end_index);

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
        buffer.push_str("\r\n");

        Ok(())
    }

    fn goto_line(&mut self, cmd: &str) {
        let num_str = cmd.trim();

        if let Ok(number) = num_str.parse::<usize>() {
            if number < 1 || number > self.lines.len() {
                self.show_status(&self.strong(&format!(
                    "{} is out of range: [1..{}]",
                    number,
                    self.lines.len()
                )));
            } else {
                self.state.current_line = number.saturating_sub(1);
            }
        } else {
            self.show_status(&self.strong("Invalid line number"));
        }
    }

    fn last_page(&mut self) {
        if self.lines.len() == 0 {
            self.state.current_line = 0;
        } else {
            self.state.current_line = self.lines.len().saturating_sub(self.screen_height);
        }
    }

    fn next_line(&mut self) {
        if self.state.current_line < self.lines.len().saturating_sub(1) {
            self.state.current_line += 1;
        }
    }

    fn next_page(&mut self) {
        let new_line =
            (self.state.current_line + self.screen_height).min(self.lines.len().saturating_sub(1));
        if new_line > self.state.current_line {
            self.state.current_line = new_line;
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

    fn search(&mut self, query: &str, forward: bool) -> io::Result<bool> {
        // Ensure the searched pattern is visible if found.
        let mut adjust_horizontal_scroll = |pos: usize| {
            if pos + query.len() >= self.screen_width {
                self.state.horizontal_scroll =
                    pos.saturating_sub(self.screen_width) + query.len() + self.line_num_width + 2;
            } else {
                self.state.horizontal_scroll = 0;
            }
        };

        let mut found = false;
        let mut interrupted = false;

        let (iter, next): (Box<dyn Iterator<Item = usize>>, Box<dyn Fn(usize) -> usize>) =
            if forward {
                (
                    Box::new(self.state.search_start_index..self.lines.len()),
                    Box::new(|i: usize| i + 1),
                )
            } else {
                (
                    Box::new((0..self.state.search_start_index).rev()),
                    Box::new(|i: usize| i.saturating_sub(1)),
                )
            };

        for i in iter {
            if Scope::is_interrupted() {
                interrupted = true;
                break;
            }

            if let Some(pos) = self.lines.get(i).and_then(|s| s.find(query)) {
                found = true;
                self.state.current_line = i;

                // Save index for repeating last search
                self.state.search_start_index = next(i);

                adjust_horizontal_scroll(pos);
                break;
            }
        }

        if !found {
            let message = if interrupted {
                Cow::Borrowed("Search aborted")
            } else {
                Cow::Owned(format!("Pattern not found: {}", query))
            };
            self.state.status_line = Some(self.strong(&message));
        }

        Ok(found)
    }

    fn repeat_search(&mut self) -> io::Result<bool> {
        if let Some(query) = self.state.last_search.clone() {
            let direction = self.state.last_search_direction;
            self.search(&query, direction)
        } else {
            Ok(false)
        }
    }

    fn run(&mut self) -> io::Result<FileAction> {
        let mut stdout = io::stdout();
        let _raw_mode = prompt::RawMode::new()?;
        execute!(stdout, EnterAlternateScreen, cursor::MoveTo(0, 0),)?;

        let mut action = FileAction::None;
        let mut buffer = String::with_capacity(self.screen_width * self.screen_height);

        self.display_page(&mut stdout, &mut buffer)?;

        while matches!(action, FileAction::None) {
            let mut state = self.state.clone();

            let event = event::read()?;

            if let Event::Resize(w, h) = event {
                self.screen_width = w.into();
                self.screen_height = h.saturating_sub(1).into();
                state.redraw = true;
            } else if let Event::Key(key_event) = event {
                if key_event.kind == KeyEventKind::Press {
                    action = self.process_key_code(key_event.code, &mut state, &mut stdout)?;
                }
            }
            if state.redraw || self.state != state {
                self.display_page(&mut stdout, &mut buffer)?;
            }
        }
        execute!(stdout, LeaveAlternateScreen)?;
        Ok(action)
    }

    fn process_key_code(
        &mut self,
        key_code: KeyCode,
        state: &mut ViewerState,
        stdout: &mut io::Stdout,
    ) -> io::Result<FileAction> {
        let mut action = FileAction::None;

        match key_code {
            KeyCode::F(1) => self.show_help(),
            KeyCode::Char('h') => self.show_help(),
            KeyCode::Char(':') => {
                let cmd = self.prompt_for_command(":")?;
                if cmd == "n" {
                    action = FileAction::NextFile;
                } else if cmd == "p" {
                    action = FileAction::PrevFile;
                } else if cmd == "q" {
                    action = FileAction::Quit;
                } else if cmd.is_empty() {
                    state.redraw = true;
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

                let (prompt, forward) = if key_code == KeyCode::Char('/') {
                    ("Search forward: ", true)
                } else {
                    ("Search backward: ", false)
                };

                let query = self.prompt_for_command(&prompt)?;
                if query.is_empty() {
                    state.redraw = true;
                } else {
                    // Search from the current line
                    self.state.search_start_index = if forward {
                        self.state.current_line
                    } else {
                        self.state.current_line + self.screen_height
                    };

                    if self.search(&query, forward)? {
                        self.state.last_search = Some(query);
                        self.state.last_search_direction = forward;
                    } else {
                        state.redraw = true;
                    }
                }
            }
            KeyCode::Char('n') => {
                if !self.repeat_search()? {
                    state.redraw = true;
                }
            }
            KeyCode::Char('l') => {
                self.state.show_line_numbers = !self.state.show_line_numbers;
            }
            _ => {}
        }

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

    /// Print a message at the bottom of the screen on the "status" line
    fn show_status(&mut self, message: &str) {
        self.state.status_line = Some(self.strong(message))
    }

    /// Show temporary hints on the last ("status") line.
    fn show_help(&mut self) {
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

        self.show_status(&help_text)
    }

    fn strong<'a>(&self, s: &'a str) -> String {
        format!("\x1b[7m{}\x1b[0m", s)
    }
}

struct Less {
    flags: CommandFlags,
}

impl Less {
    fn new() -> Self {
        let mut flags = CommandFlags::with_follow_links();
        flags.add_flag('n', "number", "Number output lines");
        Self { flags }
    }
}

impl Exec for Less {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let filenames = flags.parse(scope, args)?;
        if flags.is_present("help") {
            println!("Usage: {} [OPTION]... [FILE]...", name);
            println!("View FILE(s) or the standard input (stdin) in a pager.");
            println!("\nUser Interaction:");
            println!("  Navigation:");
            println!("    {:<20} {}", "Up Arrow", "Move one line up.");
            println!("    {:<20} {}", "Down Arrow", "Move one line down.");
            println!("    {:<20} {}", "Left Arrow", "Scroll horizontally left.");
            println!("    {:<20} {}", "Right Arrow", "Scroll horizontally right.");
            println!("    {:<20} {}", "PageUp", "Go to the previous page.");
            println!("    {:<20} {}", "b", "Go to the previous page.");
            println!("    {:<20} {}", "PageDown", "Go to the next page.");
            println!("    {:<20} {}", "f", "Go to the next page.");
            println!("    {:<20} {}", "Space", "Go to the next page.");
            println!("    {:<20} {}", "G", "Go to the last page.");
            println!("    {:<20} {}", ":N", "Go to line number N (1-based).");
            println!("    {:<20} {}", ":n", "Load the next file.");
            println!("    {:<20} {}", ":p", "Load the previous file.");
            println!("    {:<20} {}", ":q", "Quit the viewer.");
            println!("    {:<20} {}", "q", "Quit the viewer.");
            println!("\n  Search:");
            println!("    {:<20} {}", "/", "Search forward.");
            println!("    {:<20} {}", "?", "Search backward.");
            println!(
                "    {:<20} {}",
                "n", "Repeat the last search (preserving the direction)."
            );
            println!("    {:<20} {}", "Esc", "Clear the search.");
            println!("\n  Miscellaneous:");
            println!(
                "    {:<20} {}",
                "l", "Toggle line numbering for the current file."
            );
            println!(
                "    {:<20} {}",
                "h", "Show hints at the bottom of the screen."
            );
            println!(
                "    {:<20} {}",
                "F1", "Show hints at the bottom of the screen."
            );

            return Ok(Value::success());
        }

        let follow = flags.is_present("follow-links");

        if filenames.is_empty() {
            run_viewer(scope, &flags, None, None).map_err(|e| e.to_string())?;
        } else {
            let mut i: usize = 0;
            loop {
                let filename = filenames.get(i).unwrap();
                let path = Path::new(filename)
                    .resolve(follow)
                    .map_err(|e| format_error(&scope, filename, args, e))?;

                match run_viewer(
                    scope,
                    &flags,
                    Some(&path),
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

fn run_viewer(
    scope: &Arc<Scope>,
    flags: &CommandFlags,
    path: Option<&Path>,
    file_info: Option<String>,
) -> io::Result<FileAction> {
    let mut viewer = Viewer::new(scope, file_info, path)?;

    viewer.state.show_line_numbers = flags.is_present("number");
    viewer.run()
}

#[ctor::ctor]
fn register() {
    register_command(ShellCommand {
        name: "less".to_string(),
        inner: Arc::new(Less::new()),
    });
}
