use cmds::{get_command, list_registered_commands, Exec};
use console::Term;
use directories::UserDirs;
use eval::{Interp, Value, KEYWORDS};
use prompt::PromptBuilder;
use rustyline::completion::{self, FilenameCompleter};
use rustyline::error::ReadlineError;
use rustyline::highlight::MatchingBracketHighlighter;
use rustyline::history::{DefaultHistory, SearchDirection};
use rustyline::{Context, Editor, Helper, Highlighter, Hinter, Validator};
use scope::Scope;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Cursor};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::{env, usize};

#[macro_use]
mod macros;

mod cmds;
mod eval;
mod prompt;
mod scope;
mod symlnk;
mod testcmds;
mod testeval;
mod utils;

#[derive(Helper, Highlighter, Hinter, Validator)]
struct CmdLineHelper {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    #[rustyline(Highlighter)]
    highlighter: MatchingBracketHighlighter,
    scope: Rc<Scope>,
}

impl CmdLineHelper {
    fn new(scope: Rc<Scope>) -> Self {
        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            scope: Rc::clone(&scope),
        }
    }

    fn keywords(&self) -> Vec<String> {
        list_registered_commands(false)
            .into_iter()
            .chain(KEYWORDS.iter().map(|s| s.to_string()))
            .collect()
    }

    // https://github.com/kkawakam/rustyline/blob/master/src/hint.rs#L66
    fn get_history_matches(&self, line: &str, pos: usize, ctx: &Context<'_>) -> HashSet<String> {
        let mut candidates = HashSet::new();
        let history_len = ctx.history().len();

        for index in (0..history_len).rev() {
            if let Ok(Some(sr)) = ctx.history().get(index, SearchDirection::Forward) {
                if sr.entry.starts_with(line) {
                    candidates.insert(sr.entry[pos..].to_owned());
                }
            }
        }

        candidates
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

#[cfg(windows)]
/// The rustyline file auto-completer does not recognize WSL symbolic links
/// (because the standard fs lib does not support them). This function implements some
/// rudimentary support by matching the file_name prefix (not dealing with quotes and
/// escapes at this time).
fn match_path_prefix(word: &str, candidates: &mut Vec<completion::Pair>) {
    use crate::symlnk::SymLink;

    if word.ends_with("..") {
        return; // do not navigate the parent dir
    }
    let path = std::path::Path::new(word);
    let mut name = path.file_name().unwrap_or_default().to_string_lossy();
    let cwd = env::current_dir().unwrap_or(PathBuf::default());
    let mut dir = path.parent().unwrap_or(&cwd).resolve().unwrap_or_default();

    if word.ends_with("\\") {
        if let Ok(resolved) = path.resolve() {
            if resolved.exists() {
                dir = resolved;
                name = std::borrow::Cow::Borrowed("");
            }
        }
    }
    if let Ok(read_dir) = &mut fs::read_dir(&dir) {
        for entry in read_dir {
            if let Ok(dir_entry) = &entry {
                let file_name = &dir_entry.file_name();

                if file_name.to_string_lossy().starts_with(name.as_ref()) {
                    let display = if dir == cwd {
                        file_name.to_string_lossy().to_string()
                    } else {
                        dir.join(file_name).to_string_lossy().to_string()
                    };
                    let replacement = if path.resolve().unwrap_or(path.to_path_buf()).is_dir() {
                        format!("{}\\", display)
                    } else {
                        display.clone()
                    };

                    candidates.push(completion::Pair {
                        display,
                        replacement,
                    })
                }
            }
        }
    }
}

#[cfg(windows)]
fn match_wsl_symlinks(
    line: &str,
    word: &str,
    pos: &mut usize,
    candidates: &mut Vec<completion::Pair>,
) {
    if !word.is_empty() {
        if let Some(i) = line.to_lowercase().find(word) {
            *pos = i;
            match_path_prefix(word, candidates);
        }
    }
}

#[cfg(not(windows))]
fn match_wsl_symlinks(_: &str, _: &str, _: &mut usize, _: &mut Vec<completion::Pair>) {}

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
            let candidates = self.get_history_matches(&line[1..], pos - 1, ctx);
            let completions: Vec<Self::Candidate> = candidates
                .into_iter()
                .map(|entry| Self::Candidate {
                    display: format!("{}{}", &line[1..], entry),
                    replacement: format!("{}{}", &line, entry),
                })
                .collect();

            return Ok((0, completions));
        }

        // Expand keywords and builtin commands.
        let mut keywords = vec![];
        let mut kw_pos = pos;

        let (head, tail) = split_delim(line);

        if tail.starts_with("~") {
            // TODO: revisit; this may conflict with the rustyline built-in TAB completion, which
            // uses home_dir, while here the value of $HOME is used (and the user can change it).
            if let Some(v) = self.scope.lookup("HOME") {
                keywords.push(completion::Pair {
                    display: String::default(),
                    replacement: format!("{}{}{}", head, v.value().as_str(), &tail[1..]),
                });
                kw_pos = 0;
            }
        } else if tail.starts_with("$") {
            // Expand variables
            kw_pos -= tail.len();
            keywords.extend(self.scope.lookup_starting_with(&tail[1..]).iter().map(|k| {
                Self::Candidate {
                    replacement: format!("${}", k),
                    display: format!("${}", k),
                }
            }));
        } else {
            let tok = head.split_ascii_whitespace().next();

            if tok.is_none() || tok.is_some_and(|tok| get_command(&tok).is_none()) {
                // Expand keywords and commands if the line does not start with a command
                kw_pos = 0;

                for kw in self.keywords() {
                    if kw.to_lowercase().starts_with(&tail) {
                        let repl = format!("{}{} ", head, kw);
                        keywords.push(completion::Pair {
                            display: repl.clone(),
                            replacement: repl,
                        });
                    }
                }
            }
        }

        match_wsl_symlinks(line, &tail, &mut kw_pos, &mut keywords);

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
    wait: bool,
    interp: Interp,
    home_dir: Option<PathBuf>,
    history_path: Option<PathBuf>,
    profile: Option<PathBuf>,
    edit_config: rustyline::config::Config,
    prompt_builder: prompt::PromptBuilder,
}

/// Search history in reverse for entry that starts with &line[1..]
fn search_history<H: Helper>(rl: &Editor<H, DefaultHistory>, line: &str) -> Option<String> {
    let search = &line[1..];
    rl.history()
        .iter()
        .rev()
        .find(|entry| entry.starts_with(search))
        .cloned()
}

impl Shell {
    fn new() -> Self {
        #[cfg(not(test))]
        {
            ctrlc::set_handler(|| {
                INTERRUPT.store(true, SeqCst);
            })
            .expect("Error setting Ctrl+C handler");
        }

        let interp = Interp::new();
        let scope = interp.global_scope();

        Self {
            source: None,
            interactive: true,
            wait: false,
            interp,
            home_dir: None,
            history_path: None,
            profile: None,
            edit_config: rustyline::Config::builder()
                .edit_mode(rustyline::EditMode::Emacs)
                .behavior(rustyline::Behavior::PreferTerm)
                .completion_type(rustyline::CompletionType::List)
                .history_ignore_dups(true)
                .unwrap()
                .max_history_size(1024)
                .unwrap()
                .build(),
            prompt_builder: PromptBuilder::with_scope(&scope),
        }
    }

    /// Retrieve the path to the file where history is saved. Set profile path.
    fn init_interactive_mode(&mut self) -> Result<&PathBuf, String> {
        assert!(self.history_path.is_none());
        let base_dirs =
            UserDirs::new().ok_or_else(|| "Failed to get base directories".to_string())?;

        let mut path = base_dirs.home_dir().to_path_buf();

        assert!(self.home_dir.is_none());
        self.set_home_dir(&path);

        path.push(".mysh");

        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create .mysh directory: {}", e))?;

        self.profile = Some(path.join("profile"));
        path.push("history.txt");

        // Create the file if it doesn't exist
        if !path.exists() {
            File::create(&path).map_err(|e| format!("Failed to create history file: {}", e))?;
        }

        self.history_path = Some(path.clone());
        self.interp.set_var("HISTORY", path.display().to_string());

        Ok(self.history_path.as_ref().unwrap())
    }

    /// Populate global scope with argument variables.
    /// Return new child scope.
    fn new_top_scope(&self) -> Rc<Scope> {
        let scope = &self.interp.global_scope();
        // Number of args (except $0)
        scope.insert(
            "#".to_string(),
            Value::Int(env::args().count().saturating_sub(1) as _),
        );
        // All args except $0
        scope.insert(
            "@".to_string(),
            Value::Str(Rc::new(
                env::args().skip(1).collect::<Vec<String>>().join(" "),
            )),
        );
        // Interpreter pid
        scope.insert("$".to_string(), Value::Int(std::process::id() as _));
        // $0, $1, ...
        for (i, arg) in env::args().enumerate() {
            scope.insert(format!("{}", i), Value::Str(Rc::new(arg)));
        }

        Scope::new(Some(Rc::clone(&scope)))
    }

    fn prompt(&mut self) -> &str {
        &self.prompt_builder.prompt()
    }

    fn read_lines<R: BufRead>(&mut self, mut reader: R) -> Result<(), String> {
        if self.interactive {
            println!("Welcome to mysh {}", env!("CARGO_PKG_VERSION"));

            // Set up rustyline
            let mut rl = CmdLineEditor::with_config(self.edit_config)
                .map_err(|e| format!("Failed to create editor: {}", e))?;
            let h = CmdLineHelper::new(self.interp.global_scope());
            rl.set_helper(Some(h));
            rl.load_history(&self.init_interactive_mode()?).unwrap();

            self.source_profile()?; // source ~/.mysh/profile if found

            if !Term::stdout().features().colors_supported() {
                self.interp
                    .global_scope()
                    .insert("NO_COLOR".to_string(), Value::Int(1));
            }

            while !self.interp.quit {
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
                                self.eval(&history_entry);
                            } else {
                                println!("No match.");
                            }
                        } else {
                            rl.add_history_entry(line.as_str())
                                .map_err(|e| e.to_string())?;

                            self.save_history(&mut rl)?;
                            self.eval(&line);
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
                    self.eval(&script);
                }
                Err(e) => return Err(format!("Failed to read input: {}", e)),
            }
        }
        Ok(())
    }

    fn save_history(&mut self, rl: &mut CmdLineEditor) -> Result<(), String> {
        let hist_path = self.history_path.as_ref().unwrap();
        rl.save_history(&hist_path)
            .map_err(|e| format!("Could not save {}: {}", hist_path.to_string_lossy(), e))
    }

    fn set_home_dir(&mut self, path: &PathBuf) {
        self.home_dir = Some(path.clone());
        let home_dir = path.to_string_lossy().to_string();
        self.interp.set_var("HOME", home_dir);
    }

    fn show_result(&self, value: &eval::Value) {
        match value {
            Value::Str(s) => {
                eprintln!("Command not found: {}", s);
                let cmds = list_registered_commands(false);
                if let Some(near) = cmds.iter().min_by_key(|&item| strsim::levenshtein(item, s)) {
                    let scope = self.interp.global_scope();
                    eprintln!("Did you mean '{}'?", scope.err_str(near));
                }
            }
            _ => println!("{}", value),
        }
    }

    fn source_profile(&self) -> Result<(), String> {
        // Source the ~/.mysh/profile if found
        if let Some(profile) = &self.profile {
            if profile.exists() {
                let scope = self.new_top_scope();
                let eval = get_command("eval").unwrap();
                eval.exec(
                    "eval",
                    &vec![profile.display().to_string(), "--source".to_string()],
                    &scope,
                )?;
            }
        }
        Ok(())
    }

    fn eval(&mut self, input: &String) {
        INTERRUPT.store(false, SeqCst);
        let scope = self.new_top_scope();

        match &self.interp.eval(input, Some(Rc::clone(&scope))) {
            Ok(value) => {
                // Did the expression eval result in running a command? Check for errors.
                if let Value::Stat(status) = &value {
                    if let Err(e) = &status.borrow().result {
                        e.show(&scope, input);
                        return;
                    }
                } else if self.interactive {
                    self.show_result(value);
                }
            }
            Err(e) => {
                e.show(&scope, input);
                if !self.interactive && !self.wait {
                    std::process::exit(500);
                }
            }
        }
    }

    fn eval_input(&mut self) -> Result<(), String> {
        if let Some(reader) = self.source.take() {
            self.read_lines(reader)
        } else {
            panic!("No input source")
        }
    }
}

pub fn current_dir() -> Result<String, String> {
    match env::current_dir() {
        Ok(path) => Ok(path.to_path_buf().to_string_lossy().to_string()),
        Err(e) => Err(format!("Error getting current directory: {}", e)),
    }
}

fn parse_cmd_line() -> Result<Shell, String> {
    let mut shell = Shell::new();

    let args: Vec<String> = env::args().collect();
    for (i, arg) in args.iter().enumerate().skip(1) {
        if arg.starts_with("-") {
            if arg == "-c" || arg == "-k" {
                if !shell.interactive {
                    Err("Cannot specify -c command and scripts at the same time")?;
                }
                shell.source = Some(Box::new(Cursor::new(format!(
                    "{}",
                    args[i + 1..].join(" ")
                ))));
                shell.interactive = false;
                if arg == "-k" {
                    shell.wait = true;
                    shell
                        .interp
                        .global_scope()
                        .insert("NO_COLOR".to_string(), eval::Value::Int(1));
                }
                break;
            }
        } else {
            let file = File::open(&arg).map_err(|e| format!("{}: {}", arg, e))?;
            shell.source = Some(Box::new(BufReader::new(file)));
            shell.interactive = false;
            shell.interp.set_file(Some(Rc::new(arg.to_owned())));
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
        Ok(shell) => {
            match shell.eval_input() {
                Err(e) => {
                    eprintln!("{}", e);
                }
                Ok(_) => {}
            }

            if shell.wait {
                prompt::read_input("\nPress Enter to continue... ").unwrap_or(String::default());
            }
        }
    }
    Ok(())
}
