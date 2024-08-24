use crate::scope::Scope;
use colored::Colorize;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
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

/// Retrieves the current username, checking USER, then USERNAME.
fn get_username() -> String {
    env::var("USER")
        .or_else(|_| env::var("USERNAME"))
        .unwrap_or_default()
}

/// Retrieves the current hostname, checking HOSTNAME, then USERDOMAIN, then COMPUTERNAME.
fn get_hostname() -> String {
    env::var("HOSTNAME")
        .or_else(|_| env::var("USERDOMAIN"))
        .or_else(|_| env::var("COMPUTERNAME"))
        .or_else(|_| env::var("NAME"))
        .unwrap_or_default()
}

/// Retrieves the current directory as a string.
fn get_current_dir() -> String {
    env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("/"))
        .display()
        .to_string()
}

/// Constructs a prompt from a bash-like spec.
pub fn construct_prompt(spec: &str) -> String {
    let mut prompt = String::new();
    let mut chars = spec.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(next_ch) = chars.next() {
                match next_ch {
                    'u' => prompt.push_str(&get_username()),    // Username
                    'h' => prompt.push_str(&get_hostname()),    // Hostname
                    'w' => prompt.push_str(&get_current_dir()), // Current directory
                    '$' => prompt.push(if get_username() == "root" { '#' } else { '$' }), // Show `#` if root, else `$`
                    _ => prompt.push_str(&format!("\\{}", next_ch)), // Handle unknown sequences
                }
            }
        } else {
            prompt.push(ch); // Regular characters
        }
    }

    prompt
}

/// Converts a DOS cmd.exe prompt spec to a Bash-like prompt spec
pub fn convert_dos_prompt_spec(dos_spec: &str) -> String {
    let mut bash_spec = String::new();
    let mut chars = dos_spec.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' {
            if let Some(next_ch) = chars.next() {
                match next_ch {
                    'U' => bash_spec.push_str("\\u"), // Current use
                    'P' => bash_spec.push_str("\\w"), // Current working directory
                    'G' => bash_spec.push('>'),       // '>' character
                    'N' => bash_spec.push_str("\\h"), // Hostname (closest equivalent)
                    'V' => bash_spec.push_str("\\v"), // Bash version
                    'D' => bash_spec.push_str("\\d"), // Date
                    _ => bash_spec.push_str(&format!("${}", next_ch)), // Unknown code, preserve it
                }
            }
        } else {
            bash_spec.push(ch); // Regular characters
        }
    }

    bash_spec
}

// Unit tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_dos_to_bash_spec() {
        assert_eq!(convert_dos_prompt_spec("$P$G"), "\\w>"); // Path followed by '>'
        assert_eq!(convert_dos_prompt_spec("$N$G"), "\\h>"); // Hostname followed by '>'
        assert_eq!(convert_dos_prompt_spec("($P)"), "(\\w)"); // Path enclosed in parentheses
        assert_eq!(convert_dos_prompt_spec("$V"), "\\v"); // Bash version
        assert_eq!(convert_dos_prompt_spec("$D"), "\\d"); // Date
        assert_eq!(convert_dos_prompt_spec("Hello $P"), "Hello \\w"); // Mixed text and spec
        assert_eq!(convert_dos_prompt_spec("$X"), "$X"); // Unmapped code preserved
    }

    #[test]
    fn test_construct_prompt() {
        // Get real environment variables and current directory
        let username = get_username();
        let hostname = get_hostname();
        let current_dir = get_current_dir();

        assert_eq!(
            construct_prompt("\\u@\\h:\\w\\$ "),
            format!("{}@{}:{}$ ", username, hostname, current_dir)
        );
        assert_eq!(construct_prompt("\\w>"), format!("{}>", current_dir));
        assert_eq!(
            construct_prompt("\\h:\\w$ "),
            format!("{}:{}$ ", hostname, current_dir)
        );
        assert_eq!(construct_prompt("(\\w)"), format!("({})", current_dir));
    }
}
