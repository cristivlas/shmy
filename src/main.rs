use cmds::list_registered_commands;
use directories::UserDirs;
use eval::{Interp, Scope};
use rustyline::completion::{self, FilenameCompleter};
use rustyline::error::ReadlineError;
use rustyline::highlight::MatchingBracketHighlighter;
use rustyline::{history::DefaultHistory, Editor};
use rustyline::{Context, Helper, Highlighter, Hinter, Validator};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Cursor};
use std::path::PathBuf;
use std::rc::Rc;
mod cmds;
#[macro_use]
mod eval;

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
        let mut keywords = list_registered_commands();

        // TODO: cleaner way to populate keywords
        keywords.extend(
            ["exit", "if", "quit", "while"]
                .iter()
                .map(|s| s.to_string()),
        );

        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            keywords,
            scope: Rc::clone(&scope),
        }
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

impl completion::Completer for CmdLineHelper {
    type Candidate = completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>), ReadlineError> {
        // Use the file completer first
        let completions = self.completer.complete(line, pos, _ctx);

        if let Ok((start, v)) = completions {
            if !v.is_empty() {
                // Replace unescaped \ with \\ in each completion's replacement
                let escaped_completions: Vec<Self::Candidate> = v
                    .into_iter()
                    .map(|mut candidate| {
                        if line.contains('"') || candidate.replacement.starts_with('"') {
                            candidate.replacement = escape_backslashes(&candidate.replacement);
                        }
                        candidate
                    })
                    .collect();
                return Ok((start, escaped_completions));
            }
        }

        // Expand keywords and builtin commands
        let mut keywords = vec![];
        let mut ret_pos = pos;

        if line.ends_with("~") {
            // TODO: revisit; this may conflict with the rustyline built-in TAB completion, which
            // uses home_dir, while here the value of $HOME is used (and the user can change it).
            if let Some(v) = self.scope.lookup_value("HOME") {
                keywords.push(completion::Pair {
                    display: v.to_string(),
                    replacement: v.to_string(),
                });
                ret_pos -= 1;
            }
        } else {
            ret_pos = 0;
            for cmd in &self.keywords {
                // Only add to completions if the command starts with the input but is not exactly the same
                if cmd.starts_with(&line[..pos]) && cmd != &line[..pos] {
                    keywords.push(completion::Pair {
                        display: cmd.to_string(),
                        replacement: format!("{} ", cmd),
                    });
                }
            }
        }

        Ok((ret_pos, keywords))
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

impl Shell {
    fn new(source: Option<Box<dyn BufRead>>, interactive: bool, interp: Interp) -> Self {
        Self {
            source,
            interactive,
            interp,
            home_dir: None,
            history_path: None,
            edit_config: rustyline::Config::builder()
                .edit_mode(rustyline::EditMode::Emacs)
                .behavior(rustyline::Behavior::PreferTerm)
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

            self.history_path = Some(path);
        }
        Ok(self.history_path.as_ref().unwrap())
    }

    fn set_home_dir(&mut self, path: &PathBuf) {
        self.home_dir = Some(path.clone());
        debug_print!(&self.home_dir);
        let home_dir = path.to_string_lossy().to_string();
        self.interp.set_home_dir(&home_dir);
        env::set_var("HOME", home_dir);
    }

    fn save_history(&mut self, rl: &mut CmdLineEditor) -> Result<(), String> {
        let hist_path = self.get_history_path()?;
        if let Err(e) = rl.save_history(&hist_path) {
            Err(format!(
                "Could not save {}: {}",
                hist_path.to_string_lossy(),
                e
            ))?;
        }
        Ok(())
    }

    fn read_input(&mut self) -> Result<(), String> {
        if let Some(reader) = self.source.take() {
            self.read_lines(reader)
        } else {
            Err("Input source is unexpectedly None".to_string())
        }
    }

    fn read_lines<R: BufRead>(&mut self, mut reader: R) -> Result<(), String> {
        let mut quit = false;
        if self.interactive {
            let mut rl = CmdLineEditor::with_config(self.edit_config)
                .map_err(|e| format!("Failed to create editor: {}", e))?;
            let h = CmdLineHelper::new(self.interp.get_scope());
            rl.set_helper(Some(h));
            rl.load_history(&self.get_history_path()?).unwrap();

            while !quit {
                let readline = rl.readline(self.prompt());
                match readline {
                    Ok(line) => {
                        rl.add_history_entry(line.as_str()).unwrap();
                        self.eval(&mut quit, &line);
                    }
                    Err(ReadlineError::Interrupted) => {
                        println!("Type \"quit\" or \"exit\" to leave the shell.");
                    }
                    Err(err) => {
                        Err(format!("Readline error: {}", err))?;
                    }
                }
            }
            self.save_history(&mut rl)?;
        } else {
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
        match self.interp.eval(quit, input) {
            Ok(result) => {
                debug_print!(&result);
            }
            Err(s) => {
                eprintln!("{}.", s);
            }
        }
    }
}

pub fn current_dir() -> Result<String, String> {
    match env::current_dir() {
        Ok(path) => Ok(path.to_path_buf().to_string_lossy().to_string()),
        Err(e) => Err(format!("Error getting current directory: {}", e)),
    }
}

fn main() -> Result<(), ()> {
    match &mut parse_cmd_line() {
        Err(e) => {
            eprint!("Command line error: {}.", e);
        }
        Ok(shell) => match shell.read_input() {
            Err(e) => eprintln!("{}.", e),
            _ => {},
        },
    }
    Ok(())
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
