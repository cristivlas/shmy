use directories::BaseDirs;
use eval::Interp;
use rustyline::completion::{self, FilenameCompleter};
use rustyline::error::ReadlineError;
use rustyline::highlight::MatchingBracketHighlighter;
use rustyline::hint::HistoryHinter;
use rustyline::validate::MatchingBracketValidator;
use rustyline::{history::DefaultHistory, Editor};
use rustyline::{Completer, Context, Helper, Highlighter, Hinter, Validator};
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Cursor};
use std::path::PathBuf;

mod cmds;
#[macro_use]
mod eval;

// Wrap FilenameCompleter for future customizations
struct CmdLineCompleter {
    file_completer: FilenameCompleter,
}

impl CmdLineCompleter {
    pub fn new() -> Self {
        Self {
            file_completer: FilenameCompleter::new(),
        }
    }
}

impl completion::Completer for CmdLineCompleter {
    type Candidate = completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>), ReadlineError> {
        let completions = self.file_completer.complete(line, pos, _ctx)?;
        Ok(completions)
    }
}

#[derive(Helper, Completer, Highlighter, Hinter, Validator)]
struct CmdLineHelper {
    #[rustyline(Completer)]
    completer: CmdLineCompleter,
    #[rustyline(Highlighter)]
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
}

type CmdLineEditor = Editor<CmdLineHelper, DefaultHistory>;

struct Shell {
    source: Option<Box<dyn BufRead>>,
    interactive: bool,
    interp: Interp,
}

impl Shell {
    fn new(source: Option<Box<dyn BufRead>>, interactive: bool, interp: Interp) -> Self {
        Self {
            source, interactive, interp,
        }
    }

    fn get_history_path(&self) -> Result<PathBuf, String> {
        let base_dirs =
            BaseDirs::new().ok_or_else(|| "Failed to get base directories".to_string())?;

        let mut path = base_dirs.home_dir().to_path_buf();
        path.push(".mysh");

        fs::create_dir_all(&path)
            .map_err(|e| format!("Failed to create .mysh directory: {}", e))?;

        path.push("history.txt");

        // Create the file if it doesn't exist
        if !path.exists() {
            File::create(&path).map_err(|e| format!("Failed to create history file: {}", e))?;
        }

        Ok(path)
    }

    fn save_history(&self, rl: &mut CmdLineEditor) -> Result<(), String> {
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
            // TODO: with_config
            let mut rl =
                CmdLineEditor::new().map_err(|e| format!("Failed to create editor: {}", e))?;
            let h = CmdLineHelper {
                completer: CmdLineCompleter::new(),
                highlighter: MatchingBracketHighlighter::new(),
                hinter: HistoryHinter::new(),
                validator: MatchingBracketValidator::new(),
            };
            rl.set_helper(Some(h));
            rl.load_history(&self.get_history_path()?).unwrap();

            while !quit {
                let readline = rl.readline("mysh> ");
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

fn main() -> Result<(), ()> {
    match &mut parse_cmd_line() {
        Err(e) => {
            eprint!("Command line error: {}.", e);
        }
        Ok(shell) => match shell.read_input() {
            Err(e) => eprintln!("{}.", e),
            _ => {}
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
