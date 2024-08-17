use crate::eval::Scope;
use colored::Colorize;
use lazy_static::lazy_static;
use rustyline::history::MemHistory;
use rustyline::{Config, Editor};
use std::io;
use std::rc::Rc;
use std::sync::Mutex;

#[derive(PartialEq)]
pub enum Answer {
    No,
    Yes,
    All,
    Quit,
}

// Define a static mutable variable for the Editor instance
lazy_static! {
    static ref EDITOR: Mutex<Editor<(), MemHistory>> = {
        let cfg = Config::builder()
            .behavior(rustyline::Behavior::PreferTerm)
            .color_mode(rustyline::ColorMode::Forced)
            .edit_mode(rustyline::EditMode::Emacs)
            .build();
        let editor = Editor::<(), MemHistory>::with_history(cfg, MemHistory::new())
            .expect("Failed to create editor");
        Mutex::new(editor)
    };
}

pub fn confirm(prompt: String, scope: &Rc<Scope>, many: bool) -> io::Result<Answer> {
    if scope.lookup("NO_CONFIRM").is_some() {
        return Ok(Answer::Yes);
    }

    let options = if scope.lookup("NO_COLOR").is_some() {
        if many {
            "[Y]es/[N]o/[A]ll/[Q]uit".to_string()
        } else {
            "[Y]es/[N]o".to_string()
        }
    } else {
        if many {
            format!(
                "{}es/{}o/{}ll/{}uit",
                "y".green().bold(),
                "N".red().bold(),
                "a".cyan().bold(),
                "q".yellow().bold()
            )
        } else {
            format!("{}es/{}o", "y".green().bold(), "N".red().bold())
        }
    };

    let question = format!("{}? ({}) ", prompt, options);

    // Use rustyline to read the input, to avoid issues when running
    // interpreter instances with -c as the right hand-side of a pipe
    // expression.
    let mut editor = EDITOR
        .lock()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let input = editor
        .readline(&question)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

    let answer = input.trim();

    if answer.eq_ignore_ascii_case("y") {
        return Ok(Answer::Yes);
    } else if many {
        if answer.eq_ignore_ascii_case("a") {
            return Ok(Answer::All);
        } else if answer.eq_ignore_ascii_case("q") {
            return Ok(Answer::Quit);
        }
    }
    Ok(Answer::No)
}

/// Used by sudo implementation on Windows
#[cfg(windows)]
pub fn read_password(prompt: &str) -> io::Result<String> {
    rpassword::prompt_password(prompt)
}
