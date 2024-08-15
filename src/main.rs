use cmds::{get_command, list_registered_commands};
use directories::UserDirs;
use eval::{EvalError, Interp, Scope, KEYWORDS};
use rustyline::completion::{self, FilenameCompleter};
use rustyline::error::ReadlineError;
use rustyline::highlight::MatchingBracketHighlighter;
use rustyline::history::{DefaultHistory, SearchDirection};
use rustyline::{Context, Editor, Helper, Highlighter, Hinter, Validator};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Cursor};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

#[macro_use]
mod macros;

mod cmds;
mod eval;
mod prompt;
mod utils;
mod testeval;

#[derive(Helper, Highlighter, Hinter, Validator)]
struct CmdLineHelper {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    #[rustyline(Highlighter)]
    highlighter: MatchingBracketHighlighter,
    keywords: Vec<String>,
    scope: Rc<Scope>,
}

impl CmdLineHelper {
    fn new(scope: Rc<Scope>) -> Self {
        let mut keywords = list_registered_commands(false);

        keywords.extend(KEYWORDS.iter().map(|s| s.to_string()));

        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            keywords,
            scope: Rc::clone(&scope),
        }
    }

    // https://github.com/kkawakam/rustyline/blob/master/src/hint.rs#L66
    fn search_history(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<String> {
        let start = if ctx.history_index() == ctx.history().len() {
            ctx.history_index().saturating_sub(1)
        } else {
            ctx.history_index()
        };
        if let Some(sr) = ctx
            .history()
            .starts_with(line, start, SearchDirection::Reverse)
            .unwrap_or(None)
        {
            if sr.entry == line {
                return None;
            }
            return Some(sr.entry[pos..].to_owned());
        }
        None
    }
}

fn escape_backslashes(input: &str) -> String {
    let mut result = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            // Check if the next character is a backslash
            if chars.peek() == Some(&'\\') {
                // Keep both backslashes (skip one)
                result.push(c);
                result.push(chars.next().unwrap());
            } else {
                // Replace single backslash with double backslash
                result.push_str("\\\\");
            }
        } else {
            result.push(c);
        }
    }

    result
}

fn split_delim(line: &str) -> (String, String) {
    if let Some(pos) = line.rfind(&['\t', ' '][..]) {
        let head = line[..pos + 1].to_string();
        let tail = line[pos..].trim().to_lowercase();
        (head, tail)
    } else {
        (String::new(), line.to_lowercase())
    }
}

impl completion::Completer for CmdLineHelper {
    type Candidate = completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>), ReadlineError> {
        if pos < line.len() {
            return Ok((pos, vec![])); // Autocomplete only if at the end of the input.
        }
        // Expand !... TAB from history.
        if line.starts_with("!") {
            if let Some(entry) = self.search_history(&line[1..], pos - 1, ctx) {
                let repl = format!("{}{}", &line[1..], entry);
                return Ok((
                    0,
                    vec![Self::Candidate {
                        display: String::default(),
                        replacement: repl,
                    }],
                ));
            }
        }

        // Expand keywords and builtin commands.
        let mut keywords = vec![];
        let mut kw_pos = pos;

        let (head, tail) = split_delim(line);

        if tail.starts_with("~") {
            // TODO: revisit; this may conflict with the rustyline built-in TAB completion, which
            // uses home_dir, while here the value of $HOME is used (and the user can change it).
            if let Some(v) = self.scope.lookup_value("HOME") {
                keywords.push(completion::Pair {
                    display: String::default(),
                    replacement: format!("{}{}{}", head, v, &tail[1..]),
                });
                kw_pos = 0;
            }
        } else if tail.starts_with("$") {
            // Expand variables
            kw_pos -= tail.len();
            keywords.extend(self.scope.lookup_partial(&tail[1..]).iter().map(|k| {
                Self::Candidate {
                    replacement: format!("${}", k),
                    display: String::default(),
                }
            }));
        } else {
            let tok = head.split_ascii_whitespace().next();

            if tok.is_none() || tok.is_some_and(|tok| get_command(&tok).is_none()) {
                // Expand keywords and commands if the line does not start with a command
                kw_pos = 0;

                for kw in &self.keywords {
                    if kw.to_lowercase().starts_with(&tail) {
                        let repl = format!("{}{} ", head, kw);
                        keywords.push(completion::Pair {
                            display: String::default(),
                            replacement: repl,
                        });
                    }
                }
            }
        }

        if keywords.is_empty() {
            // Try the file completer next ...
            let completions = self.completer.complete(line, pos, ctx);

            if let Ok((start, v)) = completions {
                if !v.is_empty() {
                    // Replace unescaped \ with \\ in each completion's replacement
                    let escaped_completions: Vec<Self::Candidate> = v
                        .into_iter()
                        .map(|mut candidate| {
                            if tail.contains('"') || candidate.replacement.starts_with('"') {
                                candidate.replacement = escape_backslashes(&candidate.replacement);
                            }
                            candidate
                        })
                        .collect();
                    return Ok((start, escaped_completions));
                }
            }
        }

        Ok((kw_pos, keywords))
    }
}

type CmdLineEditor = Editor<CmdLineHelper, DefaultHistory>;

struct Shell {
    source: Option<Box<dyn BufRead>>,
    interactive: bool,
    interp: Interp,
    home_dir: Option<PathBuf>,
    history_path: Option<PathBuf>,
    edit_config: rustyline::config::Config,
    prompt: String,
}

fn search_history<H: Helper>(rl: &Editor<H, DefaultHistory>, line: &str) -> Option<String> {
    let search = &line[1..];
    rl.history()
        .iter()
        .rev()
        .find(|entry| entry.starts_with(search))
        .cloned()
}

impl Shell {
    fn new(source: Option<Box<dyn BufRead>>, interactive: bool, interp: Interp) -> Self {
        #[cfg(not(test))]
        {
            ctrlc::set_handler(|| {
                INTERRUPT.store(true, SeqCst);
            })
            .expect("Error setting Ctrl+C handler");
        }

        Self {
            source,
            interactive,
            interp,
            home_dir: None,
            history_path: None,
            edit_config: rustyline::Config::builder()
                .edit_mode(rustyline::EditMode::Emacs)
                .behavior(rustyline::Behavior::PreferTerm)
                .history_ignore_dups(true)
                .unwrap()
                .max_history_size(1024)
                .unwrap()
                .build(),
            prompt: String::default(),
        }
    }

    fn prompt(&mut self) -> &str {
        self.prompt = format!("{}> ", current_dir().unwrap());
        &self.prompt
    }

    // Retrieve the path to the file where history is saved.
    fn get_history_path(&mut self) -> Result<&PathBuf, String> {
        if self.history_path.is_none() {
            let base_dirs =
                UserDirs::new().ok_or_else(|| "Failed to get base directories".to_string())?;

            let mut path = base_dirs.home_dir().to_path_buf();

            assert!(self.home_dir.is_none());
            self.set_home_dir(&path);

            path.push(".mysh");

            fs::create_dir_all(&path)
                .map_err(|e| format!("Failed to create .mysh directory: {}", e))?;

            path.push("history.txt");

            // Create the file if it doesn't exist
            if !path.exists() {
                File::create(&path).map_err(|e| format!("Failed to create history file: {}", e))?;
            }

            self.history_path = Some(path.clone());
            self.interp
                .set_var("HISTORY", path.to_string_lossy().to_string());
        }
        Ok(self.history_path.as_ref().unwrap())
    }

    fn set_home_dir(&mut self, path: &PathBuf) {
        self.home_dir = Some(path.clone());
        let home_dir = path.to_string_lossy().to_string();
        self.interp.set_var("HOME", home_dir);
    }

    fn save_history(&mut self, rl: &mut CmdLineEditor) -> Result<(), String> {
        let hist_path = self.get_history_path()?;
        rl.save_history(&hist_path)
            .map_err(|e| format!("Could not save {}: {}", hist_path.to_string_lossy(), e))
    }

    fn read_input(&mut self) -> Result<(), String> {
        if let Some(reader) = self.source.take() {
            self.read_lines(reader)
        } else {
            panic!("No input source")
        }
    }

    fn read_lines<R: BufRead>(&mut self, mut reader: R) -> Result<(), String> {
        let mut quit = false;
        if self.interactive {
            // Set up rustyline
            let mut rl = CmdLineEditor::with_config(self.edit_config)
                .map_err(|e| format!("Failed to create editor: {}", e))?;
            let h = CmdLineHelper::new(self.interp.get_scope());
            rl.set_helper(Some(h));
            rl.load_history(&self.get_history_path()?).unwrap();

            while !quit {
                // run interactive read-evaluate loop
                let readline = rl.readline(self.prompt());
                match readline {
                    Ok(line) => {
                        if line.starts_with("!") {
                            if let Some(history_entry) = search_history(&rl, &line) {
                                eprintln!("{}", &history_entry);
                                // Make the entry found in history the most recent
                                rl.add_history_entry(&history_entry)
                                    .map_err(|e| e.to_string())?;
                                // Evaluate the line from history
                                self.eval(&mut quit, &history_entry);
                            } else {
                                println!("No match.");
                            }
                        } else {
                            rl.add_history_entry(line.as_str())
                                .map_err(|e| e.to_string())?;

                            self.save_history(&mut rl)?;
                            self.eval(&mut quit, &line);
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        eprintln!("^C");
                    }
                    Err(err) => {
                        Err(format!("Readline error: {}", err))?;
                    }
                }
            }
        } else {
            // Evaluate a script file
            let mut script: String = String::new();
            match reader.read_to_string(&mut script) {
                Ok(_) => {
                    self.eval(&mut quit, &script);
                }
                Err(e) => return Err(format!("Failed to read input: {}", e)),
            }
        }
        Ok(())
    }

    fn eval(&mut self, quit: &mut bool, input: &String) {
        INTERRUPT.store(false, SeqCst);

        match self.interp.eval(quit, input) {
            Ok(result) => {
                my_dbg!(&result);
            }
            Err(e) => {
                self.show_error(input, &e);
                if !self.interactive {
                    std::process::exit(500);
                }
            }
        }
    }

    fn show_error(&self, input: &String, e: &EvalError) {
        e.show(input);
    }
}

pub fn current_dir() -> Result<String, String> {
    match env::current_dir() {
        Ok(path) => Ok(path.to_path_buf().to_string_lossy().to_string()),
        Err(e) => Err(format!("Error getting current directory: {}", e)),
    }
}

fn parse_cmd_line() -> Result<Shell, String> {
    let mut shell = Shell::new(None, true, Interp::new());

    let args: Vec<String> = env::args().collect();
    for (i, arg) in args.iter().enumerate().skip(1) {
        if arg.starts_with("-") {
            if arg == "-c" {
                if !shell.interactive {
                    Err("cannot specify -c command and scripts at the same time")?;
                }
                shell.source = Some(Box::new(Cursor::new(format!(
                    "{}",
                    args[i + 1..].join(" ")
                ))));
                shell.interactive = false;
                break;
            }
        } else {
            let file = File::open(&arg).map_err(|e| format!("{}: {}", arg, e))?;
            shell.source = Some(Box::new(BufReader::new(file)));
            shell.interactive = false;
        }
    }

    if shell.source.is_none() {
        shell.source = Some(Box::new(BufReader::new(io::stdin())));
    }

    Ok(shell)
}

static INTERRUPT: AtomicBool = AtomicBool::new(false); // Ctrl+C pressed?

fn main() -> Result<(), ()> {
    match &mut parse_cmd_line() {
        Err(e) => {
            eprint!("Command line error: {}.", e);
        }
        Ok(shell) => match shell.read_input() {
            Err(e) => {
                eprintln!("{}", e);
            }
            Ok(_) => {}
        },
    }
    Ok(())
}
