use crate::{eval::Value, scope::Scope};
use colored::Colorize;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::env;
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

pub struct PromptBuilder {
    scope: Rc<Scope>,
    prompt: String,
}

impl PromptBuilder {
    pub fn with_scope(scope: &Rc<Scope>) -> Self {
        Self {
            scope: Rc::clone(&scope),
            prompt: String::new(),
        }
    }

    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_scope(&Scope::with_env_vars())
    }

    pub fn prompt(&mut self) -> &str {
        let spec = Self::prompt_spec(&self.scope);
        self.build(spec.as_str())
    }

    fn prompt_spec(scope: &Rc<Scope>) -> Rc<String> {
        if let Some(var) = scope.lookup("__prompt") {
            var.value().to_rc_string()
        } else {
            // Create default prompt specification and insert into the scope.
            let spec = Rc::new("\\u@\\h|\\w\\$ ".to_string());
            scope.insert("__prompt".to_string(), Value::Str(Rc::clone(&spec)));

            spec
        }
    }

    fn username(&self) -> Rc<String> {
        if let Some(var) = self
            .scope
            .lookup("USER")
            .or_else(|| self.scope.lookup("USERNAME"))
        {
            var.value().to_rc_string()
        } else {
            Rc::default()
        }
    }

    fn is_root(&self) -> bool {
        self.username().as_str() == "root"
    }

    fn push_username(&mut self) {
        self.prompt.push_str(&self.username())
    }

    fn push_hostname(&mut self) {
        if let Some(hostname) = self
            .scope
            .lookup("HOSTNAME")
            .or_else(|| self.scope.lookup("USERDOMAIN"))
            .or_else(|| self.scope.lookup("COMPUTERNAME"))
            .or_else(|| self.scope.lookup("NAME"))
        {
            self.prompt.push_str(&hostname.value().as_str());
        }
    }

    fn push_current_dir(&mut self) {
        self.prompt
            .push_str(&env::current_dir().unwrap_or_default().display().to_string());
    }

    pub fn build(&mut self, spec: &str) -> &str {
        self.prompt.clear();

        let mut chars = spec.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(next_ch) = chars.next() {
                    match next_ch {
                        'u' => self.push_username(),
                        'h' => self.push_hostname(),
                        'w' => self.push_current_dir(),
                        '$' => self.prompt.push(if self.is_root() { '#' } else { '$' }),
                        _ => {
                            self.prompt.push(next_ch);
                        }
                    }
                }
            } else {
                self.prompt.push(ch);
            }
        }

        &self.prompt
    }
}

// Unit tests
#[cfg(test)]
mod tests {
    use super::*;

    fn get_username() -> String {
        env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_default()
    }

    fn get_hostname() -> String {
        env::var("HOSTNAME")
            .or_else(|_| env::var("USERDOMAIN"))
            .or_else(|_| env::var("COMPUTERNAME"))
            .or_else(|_| env::var("NAME"))
            .unwrap_or_default()
    }

    fn get_current_dir() -> String {
        env::current_dir().unwrap_or_default().display().to_string()
    }

    #[test]
    fn test_build() {
        // Get real environment variables and current directory
        let username = get_username();
        let hostname = get_hostname();
        let current_dir = get_current_dir();

        let mut builder = PromptBuilder::new();

        assert_eq!(
            builder.build("\\u@\\h:\\w\\$ "),
            format!("{}@{}:{}$ ", username, hostname, current_dir)
        );
        assert_eq!(builder.build("\\w>"), format!("{}>", current_dir));
        assert_eq!(
            builder.build("\\h:\\w$ "),
            format!("{}:{}$ ", hostname, current_dir)
        );
        assert_eq!(builder.build("(\\w)"), format!("({})", current_dir));
    }
}
