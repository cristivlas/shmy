use crate::eval::Scope;
use colored::Colorize;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, Write};
use std::rc::Rc;

#[derive(PartialEq)]
pub enum Answer {
    No,
    Yes,
    All,
    Quit,
}

pub fn confirm(prompt: String, scope: &Rc<Scope>, one_of_many: bool) -> io::Result<Answer> {
    // Bypass confirmation?
    if scope.lookup("NO_CONFIRM").is_some() {
        return Ok(Answer::Yes);
    }

    let use_colors = scope.use_colors(&std::io::stdout());

    let options = if !use_colors {
        if one_of_many {
            "[Y]es/[N]o/[A]ll/[Q]uit".to_string()
        } else {
            "[Y]es/[N]o".to_string()
        }
    } else {
        if one_of_many {
            format!(
                "{}es/{}o/{}ll/{}uit",
                "y".bright_green().bold(),
                "N".red().bold(),
                "a".blue().bold(),
                "q".truecolor(255, 165, 0).bold() // Orange
            )
        } else {
            format!("{}es/{}o", "y".green().bold(), "N".red().bold())
        }
    };

    let question = format!("{}? ({}) ", prompt, options);

    // Open the TTY for writing the prompt
    let mut tty = open_tty_for_writing()?;
    write!(tty, "{}", question)?;
    tty.flush()?;

    enable_raw_mode()?;

    let mut input = String::new();
    loop {
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match key_event.code {
                    KeyCode::Char(c) => {
                        input.push(c);
                        write!(tty, "{}", c)?;
                        tty.flush()?;
                    }
                    KeyCode::Enter => {
                        writeln!(tty)?;
                        break;
                    }
                    KeyCode::Backspace => {
                        if !input.is_empty() {
                            input.pop();
                            write!(tty, "\x08 \x08")?;
                            tty.flush()?;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    write!(tty, "\r")?;
    disable_raw_mode()?;

    process_answer(&input, one_of_many)
}

fn process_answer(input: &str, many: bool) -> io::Result<Answer> {
    let first_char = input.trim().chars().next().map(|c| c.to_ascii_lowercase());

    match first_char {
        Some('y') => Ok(Answer::Yes),
        Some('n') => Ok(Answer::No),
        Some('a') if many => Ok(Answer::All),
        Some('q') if many => Ok(Answer::Quit),
        _ => Ok(Answer::No),
    }
}

fn open_tty_for_writing() -> io::Result<impl Write> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        OpenOptions::new().write(true).open("/dev/tty")
    }
    #[cfg(windows)]
    {
        use std::fs::OpenOptions;
        OpenOptions::new().write(true).open("CON")
    }
}
