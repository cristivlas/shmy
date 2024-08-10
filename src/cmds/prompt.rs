use colored::Colorize;
use std::io::{self, Write};

#[derive(PartialEq)]
pub enum Answer {
    No,
    Yes,
    All,
    Quit,
}

pub fn confirm(prompt: String, many: bool) -> io::Result<Answer> {
    let options = if many {
        format!(
            "{}es/{}o/{}ll/{}uit",
            "y".green().bold(),
            "N".red().bold(),
            "a".cyan().bold(),
            "q".yellow().bold()
        )
    } else {
        format!("{}es/{}o", "y".green().bold(), "N".red().bold())
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
