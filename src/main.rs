use cmds::{get_command, registered_commands, Exec};
use console::Term;
use directories::UserDirs;
use eval::{Interp, Value, KEYWORDS};
use prompt::PromptBuilder;
use rustyline::completion::{self, FilenameCompleter};
use rustyline::error::ReadlineError;
use rustyline::highlight::MatchingBracketHighlighter;
use rustyline::history::{DefaultHistory, SearchDirection};
use rustyline::{highlight::Highlighter, Context, Editor, Helper, Hinter, Validator};
use scope::Scope;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Cursor};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering::SeqCst},
    Arc, LazyLock, Mutex,
};
use std::{env, usize};
use yaml_rust::Yaml;

#[macro_use]
mod macros;

mod cmds;
mod completions;
mod eval;
mod prompt;
mod scope;
mod symlnk;
mod testcmds;
mod testeval;
mod utils;

#[derive(Helper, Hinter, Validator)]
struct CmdLineHelper {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    #[rustyline(Highlighter)]
    highlighter: MatchingBracketHighlighter,
    interp: Interp, // Interpreter instance for tab completion
    completions: Option<Yaml>,
    prompt: String,
}

impl Highlighter for CmdLineHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Cow::Borrowed(&self.prompt)
        } else {
            Cow::Borrowed(prompt)
        }
    }

    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_char(&self, line: &str, pos: usize, forced: bool) -> bool {
        self.highlighter.highlight_char(line, pos, forced)
    }
}

impl CmdLineHelper {
    fn new(scope: Arc<Scope>, completions: Option<Yaml>) -> Self {
        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            interp: Interp::new(scope),
            completions,
            prompt: String::default(),
        }
    }

    /// Complete arguments for builtin commands.
    fn complete_commands(
        &self,
        input: &str,
        pos: &mut usize,
        candidates: &mut Vec<completion::Pair>,
    ) {
        // Get registered commands. Pass false to internal_only,
        // to include cached, previously used external commands
        for name in &registered_commands(false) {
            if name.starts_with(input) {
                candidates.push(completion::Pair {
                    display: name.clone(),
                    replacement: name.clone(),
                })
            } else if input.starts_with(name) {
                if let Some(delim_pos) = input.rfind(&['\t', ' '][..]) {
                    // Complete command line flags and options for internal cmds.
                    let arg = &input[&delim_pos + 1..];
                    if !arg.starts_with("-") {
                        continue;
                    }
                    let cmd = get_command(name).unwrap();
                    for f in cmd.cli_flags() {
                        if let Some(short) = f.short {
                            let flag = format!("-{}", short);
                            if flag.starts_with(arg) {
                                candidates.push(completion::Pair {
                                    display: flag.clone(),
                                    replacement: flag,
                                })
                            }
                        }
                        let flag = format!("--{}", f.long);
                        if flag.starts_with(arg) {
                            candidates.push(completion::Pair {
                                display: flag.clone(),
                                replacement: flag,
                            })
                        }
                        if !f.takes_value && arg.starts_with("--no-") && !f.long.starts_with("no-")
                        {
                            if f.long.starts_with(&arg[5..]) {
                                let flag = format!("--no-{}", f.long);
                                candidates.push(completion::Pair {
                                    display: flag.clone(),
                                    replacement: flag,
                                })
                            }
                        }
                    }
                    if !candidates.is_empty() {
                        *pos += delim_pos + 1;
                    }
                }
            }
        }
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

    fn set_prompt(&mut self, prompt: &str) {
        self.prompt = prompt.into()
    }

    /// Completion helper. Uses the helper interpreter instance to parse
    /// and extract the tail of the input rather than just splitting at whitespace.
    /// If the parsing attempt does not work, then fail over to simple space split.
    fn get_tail<'a>(&self, input: &'a str) -> (usize, &'a str) {
        if let Some((loc, tail)) = self.interp.parse_tail(input) {
            if loc.line == 1 {
                let pos = match input.rfind(&tail) {
                    Some(pos) => pos,
                    None => std::cmp::min(loc.col.saturating_sub(1) as usize, input.len()),
                };
                return (pos, &input[pos..].trim());
            }
        }

        return (0, input);
    }
}

#[cfg(windows)]
/// The rustyline file tab-completer does not recognize WSL symbolic links
/// (because the standard fs lib does not support them). This function implements some
/// rudimentary support by matching the file_name prefix (not dealing with quotes and
/// escapes at this time).
fn match_path_prefix(word: &str, candidates: &mut Vec<completion::Pair>) {
    use crate::symlnk::SymLink;

    let path = std::path::Path::new(word);
    let mut name = path.file_name().unwrap_or_default().to_string_lossy();
    let cwd = env::current_dir().unwrap_or(PathBuf::default());
    let mut dir = path
        .parent()
        .unwrap_or(&cwd)
        .dereference()
        .unwrap_or_default()
        .into_owned();

    if word.ends_with("\\") {
        if let Ok(resolved) = path.dereference() {
            if resolved.exists() {
                dir = resolved.into();
                name = std::borrow::Cow::Borrowed("");
            }
        }
    }

    if let Ok(read_dir) = &mut fs::read_dir(&dir) {
        for entry in read_dir {
            if let Ok(dir_entry) = &entry {
                let file_name = &dir_entry.file_name();

                if file_name
                    .to_string_lossy()
                    .to_lowercase()
                    .starts_with(name.as_ref())
                {
                    let display = if dir == cwd {
                        file_name.to_string_lossy().to_string()
                    } else {
                        if dir.starts_with(&cwd) {
                            dir = dir.strip_prefix(&cwd).unwrap_or(&dir).to_path_buf();
                        }

                        dir.join(file_name).to_string_lossy().to_string()
                    };

                    let replacement = if path.dereference().unwrap_or(path.into()).is_dir() {
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
fn match_symlinks(input: &str, pos: &mut usize, candidates: &mut Vec<completion::Pair>) {
    if let Some(mut delim_pos) = input.rfind(&['\t', ' '][..]) {
        delim_pos += 1;
        match_path_prefix(&input[delim_pos..], candidates);

        if !candidates.is_empty() {
            *pos += delim_pos;
        }
    } else {
        match_path_prefix(input, candidates);
    }
}

#[cfg(not(windows))]
fn match_symlinks(_: &str, _: &mut usize, _: &mut Vec<completion::Pair>) {}

impl completion::Completer for CmdLineHelper {
    type Candidate = completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>), ReadlineError> {
        // Complete only at the end of the input.
        if pos < line.len() {
            return Ok((pos, vec![]));
        }

        // Expand ! TAB from history.
        if line.starts_with("!") {
            let candidates = self.get_history_matches(&line[1..], pos - 1, ctx);
            let completions: Vec<Self::Candidate> = candidates
                .iter()
                .map(|entry| Self::Candidate {
                    display: format!("{}{}", &line[1..], entry),
                    replacement: format!("{}{}", &line, entry),
                })
                .collect();

            return Ok((0, completions));
        }

        let (mut tail_pos, tail) = self.get_tail(line);

        let mut completions = vec![];

        if tail.starts_with("~") {
            // NOTE: this may conflict with the rustyline built-in TAB completion, which uses
            // home_dir, while here the value of the $HOME var is used (which the user can change).
            if let Some(v) = self.interp.global_scope().lookup("HOME") {
                completions.push(completion::Pair {
                    display: String::default(), // Don't care, there is only one candidate.
                    replacement: format!("{}{}", v.value().as_str(), &tail[1..]),
                });
            }
        } else if let Some(var_pos) = tail.rfind("$") {
            // Expand variables. NOTE: No variable substitution, just name expansion.
            completions.extend(
                self.interp
                    .global_scope()
                    .lookup_starting_with(&tail[var_pos + 1..])
                    .iter()
                    .map(|k| Self::Candidate {
                        replacement: format!("${}", k),
                        display: format!("${}", k),
                    }),
            );
            if !completions.is_empty() {
                tail_pos += var_pos;
            }
        } else {
            for kw in KEYWORDS {
                if kw.to_lowercase().starts_with(&tail) {
                    completions.push(completion::Pair {
                        display: kw.to_string(),
                        replacement: kw.to_string(),
                    });
                }
            }
            if completions.is_empty() {
                self.complete_commands(tail, &mut tail_pos, &mut completions);
            }

            if completions.is_empty() {
                // Custom (user-defined) command completions
                if let Some(config) = &self.completions {
                    for completion in completions::suggest(config, tail) {
                        completions.push(completion::Pair {
                            display: completion.clone(),
                            replacement: completion,
                        });
                    }
                }
            }
        }

        if completions.is_empty() {
            // Handle (Windows-native and WSL) symbolic links.
            match_symlinks(&tail, &mut tail_pos, &mut completions);
        }
        if completions.is_empty() {
            self.completer.complete(line, pos, ctx) // Rustyline path completion
        } else {
            Ok((tail_pos, completions))
        }
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
    user_dirs: UserDirs,
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
    fn new() -> Result<Self, String> {
        #[cfg(not(test))]
        {
            ctrlc::set_handler(|| {
                _ = INTERRUPT_EVENT
                    .try_lock()
                    .and_then(|mut event| Ok(event.set()))
            })
            .expect("Error setting Ctrl+C handler");
        }

        let interp = Interp::with_env_vars();
        let scope = interp.global_scope();

        let mut shell = Self {
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
            user_dirs: UserDirs::new()
                .ok_or_else(|| "Failed to get user directories".to_string())?,
        };
        shell.set_home_dir(shell.user_dirs.home_dir().to_path_buf());

        Ok(shell)
    }

    /// Retrieve the path to the file where history is saved. Set profile path.
    fn init_interactive_mode(&mut self) -> Result<(&PathBuf, Option<Yaml>), String> {
        let mut path = self.home_dir.as_ref().expect("home dir not set").clone();

        path.push(".shmy");

        // Ensure the directory exists.
        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create .shmy directory: {}", e))?;

        self.profile = Some(path.join("profile"));

        // Load custom completion file if present
        let compl_config_path = path.join("completions.yaml");
        let compl_config = if compl_config_path.exists() {
            Some(
                completions::load_config_from_file(&compl_config_path).map_err(|e| {
                    format!("Failed to load {}: {}", compl_config_path.display(), e)
                })?,
            )
        } else {
            None
        };

        // Set up command line history file
        path.push("history.txt");

        // Create the file if it doesn't exist
        if !path.exists() {
            File::create(&path).map_err(|e| format!("Failed to create history file: {}", e))?;
        }

        self.history_path = Some(path.clone());
        self.interp.set_var("HISTORY", path.display().to_string());

        Ok((self.history_path.as_ref().unwrap(), compl_config))
    }

    /// Populate global scope with argument variables.
    /// Return new child scope.
    fn new_top_scope(&self) -> Arc<Scope> {
        let scope = &self.interp.global_scope();
        // Number of args (not including $0)
        scope.insert(
            "#".to_string(),
            Value::Int(env::args().count().saturating_sub(1) as _),
        );
        // All args (not including $0)
        scope.insert(
            "@".to_string(),
            Value::Str(Arc::new(
                env::args().skip(1).collect::<Vec<String>>().join(" "),
            )),
        );
        // Interpreter process id
        scope.insert("$".to_string(), Value::Int(std::process::id() as _));
        // $0, $1, ...
        for (i, arg) in env::args().enumerate() {
            scope.insert(format!("{}", i), Value::Str(Arc::new(arg)));
        }

        Scope::with_parent(Some(Arc::clone(&scope)))
    }

    fn read_lines<R: BufRead>(&mut self, mut reader: R) -> Result<(), String> {
        if self.interactive {
            println!("Welcome to shmy {}", env!("CARGO_PKG_VERSION"));

            // Set up rustyline
            let mut rl = CmdLineEditor::with_config(self.edit_config)
                .map_err(|e| format!("Failed to create editor: {}", e))?;

            let scope = self.interp.global_scope();
            let (history_path, completion_config) = self.init_interactive_mode()?;

            rl.set_helper(Some(CmdLineHelper::new(scope, completion_config)));
            rl.load_history(history_path).unwrap();

            self.source_profile()?; // source ~/.shmy/profile if found

            if !Term::stdout().features().colors_supported() {
                self.interp
                    .global_scope()
                    .insert("NO_COLOR".to_string(), Value::Int(1));
            } else {
                //
                // The `colored`` crate contains a SHOULD_COLORIZE singleton
                // https://github.com/colored-rs/colored/blob/775ec9f19f099a987a604b85dc72ca83784f4e38/src/control.rs#L79
                //
                // If the very first command executed from our shell is redirected or piped, e.g.
                // ```ls -al | cat```
                // then the output of the command does not output to a terminal, and the 'colored' crate
                // will cache that state and never colorize for the lifetime of the shell instance.
                //
                // The line below forces SHOULD_COLORIZE to be initialized early rather than lazily.
                //
                colored::control::unset_override();
            }

            // Run interactive read-evaluate loop
            while !self.interp.quit {
                let prompt = self.prompt_builder.prompt();

                // Hack around peculiarity in Rustyline, where a prompt that contains color ANSI codes
                // needs to go through the highlighter trait in the helper. The prompt passed to readline
                // (see below) causes the Windows terminal to misbehave when it contains ANSI color codes.
                rl.helper_mut().unwrap().set_prompt(&prompt);

                // Pass prompt without ANSI codes to readline
                let readline = rl.readline(&self.prompt_builder.without_ansi());

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
                                eprintln!("No match.");
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

    fn set_home_dir(&mut self, path: PathBuf) {
        let home_dir = path.to_string_lossy().to_string();
        self.home_dir = Some(path);
        self.interp.set_var("HOME", home_dir);
    }

    fn show_result(&self, scope: &Arc<Scope>, input: &str, value: &eval::Value) {
        use strsim::levenshtein;

        if input.is_empty() {
            return;
        }
        match value {
            Value::Str(s) => {
                println!("{}", s);

                if !input.contains(" ") {
                    let cmds = registered_commands(false);
                    if let Some((near, distance)) = cmds
                        .iter()
                        .map(|item| (item, levenshtein(item, s)))
                        .min_by_key(|&(_, distance)| distance)
                    {
                        if distance < std::cmp::max(near.len(), input.len()) {
                            eprintln!(
                                "{} was evaluated as a string. Did you mean '{}'?",
                                scope.err_str(input),
                                scope.err_str(near),
                            );
                        }
                    }
                }
            }
            _ => println!("{}", value),
        }
    }

    fn source_profile(&self) -> Result<(), String> {
        // Source ~/.shmy/profile if it exists
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
        INTERRUPT_EVENT
            .try_lock()
            .and_then(|mut event| Ok(event.clear()))
            .expect("Could not reset interrupt event");

        let scope = self.new_top_scope();

        match &self.interp.eval(input, Some(Arc::clone(&scope))) {
            Ok(value) => {
                // Did the expression eval result in running a command? Check for errors.
                if let Value::Stat(status) = &value {
                    if let Err(e) = &status.borrow().result {
                        e.show(&scope, input);
                    }
                } else if self.interactive {
                    self.show_result(&scope, &input.trim(), &value);
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
    match &env::current_dir() {
        Ok(path) => Ok(path.to_string_lossy().to_string()),
        Err(e) => Err(format!("Error getting current directory: {}", e)),
    }
}

fn parse_cmd_line() -> Result<Shell, String> {
    let mut shell = Shell::new()?;

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
            shell.interp.set_file(Some(Arc::new(arg.to_owned())));
        }
    }

    if shell.source.is_none() {
        shell.source = Some(Box::new(BufReader::new(io::stdin())));
    }

    Ok(shell)
}

pub struct InterruptEvent {
    flag: AtomicBool,

    #[cfg(windows)]
    pub event: utils::win::SafeHandle,
}

impl InterruptEvent {
    fn new() -> io::Result<Self> {
        Ok(Self {
            flag: AtomicBool::new(false),

            #[cfg(windows)]
            event: utils::win::create_auto_reset_event()?,
        })
    }

    pub fn clear(&mut self) {
        self.flag.store(false, SeqCst);
    }

    pub fn is_set(&self) -> bool {
        self.flag.load(SeqCst)
    }

    pub fn set(&mut self) {
        self.flag.store(true, SeqCst);

        #[cfg(windows)]
        utils::win::set_event(&self.event);
    }
}

pub static INTERRUPT_EVENT: LazyLock<Mutex<InterruptEvent>> =
    LazyLock::new(|| Mutex::new(InterruptEvent::new().expect("Failed to create InterruptEvent")));

fn main() -> Result<(), ()> {
    match &mut parse_cmd_line() {
        Err(e) => {
            eprint!("Command line error: {}.", e);
        }
        Ok(shell) => {
            match &shell.eval_input() {
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

#[cfg(test)]
mod tests {
    use super::*;
    ///
    /// TAB completion tests.
    ///
    use completion::Completer;
    use rustyline::history::{History, MemHistory};

    fn get_completions(
        helper: &CmdLineHelper,
        input: &str,
        history: &MemHistory,
    ) -> Vec<(String, String)> {
        let pos = input.len();
        let result = helper.complete(input, pos, &Context::new(history));
        assert!(result.is_ok());

        result
            .unwrap()
            .1
            .iter()
            .map(|pair| (pair.display.clone(), pair.replacement.clone()))
            .collect()
    }

    #[test]
    fn test_complete_var() {
        let scope = Scope::new();
        scope.insert("HOME".into(), Value::from("home"));
        let helper = CmdLineHelper::new(scope, None);
        let actual_completions = get_completions(&helper, "$HO", &MemHistory::new());
        let expected_completions = vec![("$HOME".to_string(), "$HOME".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_var_arg() {
        let scope = Scope::new();
        scope.insert("HOME".into(), Value::from("home"));
        let helper = CmdLineHelper::new(scope, None);
        let actual_completions = get_completions(&helper, "echo $HO", &MemHistory::new());
        let expected_completions = vec![("$HOME".to_string(), "$HOME".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_tilde() {
        let scope = Scope::new();
        scope.insert("HOME".into(), Value::from("home"));
        let helper = CmdLineHelper::new(scope, None);
        let actual_completions = get_completions(&helper, "~", &MemHistory::new());
        let expected_completions = vec![("".to_string(), "home".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_tilde_prefix() {
        let scope = Scope::new();
        scope.insert("HOME".into(), Value::from("\\home\\bob"));
        let helper = CmdLineHelper::new(scope, None);
        let actual_completions = get_completions(&helper, "~\\Test", &MemHistory::new());
        let expected_completions = vec![("".to_string(), "\\home\\bob\\Test".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_history() {
        let helper = CmdLineHelper::new(Scope::new(), None);
        let mut history = MemHistory::new();
        history.add("foozy").unwrap();
        let actual_completions = get_completions(&helper, "!foo", &history);
        let expected_completions = vec![("foozy".to_string(), "!foozy".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_pipe() {
        let scope = Scope::new();
        scope.insert("HOME".into(), Value::from("\\home\\bob"));
        let helper = CmdLineHelper::new(scope, None);
        let actual_completions = get_completions(&helper, "ls | ~\\foo", &MemHistory::new());
        let expected_completions = vec![("".to_string(), "\\home\\bob\\foo".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_path() {
        let helper = CmdLineHelper::new(Scope::new(), None);
        let actual_completions =
            get_completions(&helper, "echo Hello && ls src/mai", &MemHistory::new());
        #[cfg(windows)]
        let expected_completions = vec![("src\\main.rs".to_string(), "src\\main.rs".to_string())];
        #[cfg(not(windows))]
        let expected_completions = vec![("main.rs".to_string(), "src/main.rs".to_string())];
        assert_eq!(actual_completions, expected_completions);
    }

    #[test]
    fn test_complete_negated_flags() {
        let helper = CmdLineHelper::new(Scope::new(), None);
        let actual_completions = get_completions(&helper, "cat  abc --no-", &MemHistory::new());
        let expected_completions = vec![
            ("--no-help".to_string(), "--no-help".to_string()),
            ("--no-number".to_string(), "--no-number".to_string()),
        ];
        assert_eq!(actual_completions, expected_completions);
    }
}
