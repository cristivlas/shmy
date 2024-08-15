use crate::eval::Scope;
use colored::Colorize;
use std::io::{self, Write};
use std::rc::Rc;

#[derive(PartialEq)]
pub enum Answer {
    No,
    Yes,
    All,
    Quit,
}

pub fn confirm(prompt: String, scope: &Rc<Scope>, many: bool) -> io::Result<Answer> {
    if scope.lookup("NO_CONFIRM").is_some() {
        // TODO: should Interp set NO_CONFIRM in non-interactive mode?
        // TODO: is SILENT a better name?
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
    print!("{}? ({}) ", prompt, options);
    io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

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
