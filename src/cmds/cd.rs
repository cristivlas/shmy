use super::{flags::CommandFlags, register_command, Exec, ShellCommand};
use crate::symlnk::SymLink;
use crate::{current_dir, eval::Value, scope::Scope};
use std::cell::RefCell;
use std::{env, path::Path, sync::Arc};

struct ChangeDir {
    stack: RefCell<Vec<String>>,
    flags: CommandFlags,
}

struct PrintWorkingDir {
    flags: CommandFlags,
}

impl ChangeDir {
    fn new() -> Self {
        let flags = CommandFlags::with_follow_links();
        Self {
            stack: RefCell::new(Vec::new()), // pushd / popd stack
            flags,
        }
    }

    fn do_chdir(&self, scope: &Arc<Scope>, follow: bool, dir: &str) -> Result<(), String> {
        let path = Path::new(dir).resolve(follow).map_err(|e| e.to_string())?;

        env::set_current_dir(&path)
            .map_err(|e| format!("Change dir to \"{}\": {}", scope.err_str(dir), e))?;
        Ok(())
    }

    fn chdir(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let parsed_args = flags.parse(scope, args)?;

        if flags.is_present("help") {
            match name {
                "cd" | "chdir" => {
                    println!("Usage: {} [DIR]", name);
                    println!("Change the current directory to DIR.");
                }
                "pushd" => {
                    println!("Usage: pushd <DIR>");
                    println!("Push the current directory onto the stack and change to DIR.");
                }
                "popd" => {
                    println!("Usage: popd");
                    println!("Pop the top directory from the stack and change to it.");
                }
                _ => unreachable!(),
            }
            println!("\nOptions:");
            print!("{}", flags.help());

            return Ok(Value::success());
        }

        let follow = flags.is_present("follow-links");
        match name {
            "cd" | "chdir" => {
                let new_dir = if parsed_args.is_empty() {
                    scope
                        .lookup_value("HOME")
                        .unwrap_or(Value::default())
                        .to_string()
                } else {
                    parsed_args.join(" ")
                };
                self.do_chdir(scope, follow, &new_dir)?
            }
            "pushd" => {
                let new_dir = if parsed_args.is_empty() {
                    return Err("pushd: no directory specified".to_string());
                } else {
                    parsed_args.join(" ")
                };
                self.stack.borrow_mut().push(current_dir()?);
                self.do_chdir(scope, follow, &new_dir)?
            }
            "popd" => {
                if self.stack.borrow().is_empty() {
                    return Err("popd: directory stack empty".to_string());
                }
                let old_dir = self.stack.borrow_mut().pop().unwrap();
                self.do_chdir(scope, follow, &old_dir)?
            }
            _ => unreachable!(),
        }

        Ok(Value::success())
    }
}

impl Exec for ChangeDir {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        self.chdir(name, args, scope)
    }
}

impl PrintWorkingDir {
    fn new() -> Self {
        let mut flags = CommandFlags::new();
        flags.add_flag('h', "help", "Display this help message");
        Self { flags }
    }
}

impl Exec for PrintWorkingDir {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Arc<Scope>) -> Result<Value, String> {
        let mut flags = self.flags.clone();
        let _ = flags.parse(scope, args)?;

        if flags.is_present("help") {
            println!("Usage: pwd");
            println!("Print the current working directory.");
            println!("\nOptions:");
            print!("{}", flags.help());
            return Ok(Value::success());
        }

        println!("{}", current_dir()?);
        Ok(Value::success())
    }
}

#[ctor::ctor]
fn register() {
    let chdir = Arc::new(ChangeDir::new());

    register_command(ShellCommand {
        name: "cd".to_string(),
        inner: Arc::clone(&chdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "pushd".to_string(),
        inner: Arc::clone(&chdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "popd".to_string(),
        inner: Arc::clone(&chdir) as Arc<dyn Exec>,
    });

    register_command(ShellCommand {
        name: "pwd".to_string(),
        inner: Arc::new(PrintWorkingDir::new()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scope::Scope;
    use std::env;

    #[test]
    fn test_cd_to_specific_dir() {
        let chdir = ChangeDir::new();
        let target_dir: String = env::current_dir().unwrap().to_string_lossy().to_string();

        let result = chdir.exec("cd", &vec![target_dir.clone()], &Scope::new());

        assert!(result.is_ok());
        assert_eq!(env::current_dir().unwrap(), Path::new(&target_dir));
    }

    #[test]
    fn test_pushd_and_popd() {
        let chdir = ChangeDir::new();
        let initial_dir = env::current_dir().unwrap();
        let new_dir = initial_dir.parent().unwrap();

        // Test pushd
        let result_pushd = chdir.exec(
            "pushd",
            &vec![new_dir.to_string_lossy().to_string()],
            &Scope::new(),
        );
        assert!(result_pushd.is_ok());
        assert_eq!(env::current_dir().unwrap(), new_dir);

        // Test popd
        let result_popd = chdir.exec("popd", &vec![], &Scope::new());
        assert!(result_popd.is_ok());
        assert_eq!(env::current_dir().unwrap(), initial_dir);
    }

    #[test]
    fn test_popd_empty_stack() {
        let chdir = ChangeDir::new();

        let result = chdir.exec("popd", &vec![], &Scope::new());

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "popd: directory stack empty".to_string()
        );
    }

    #[test]
    fn test_pwd() {
        let pwd = PrintWorkingDir::new();
        let result = pwd.exec("pwd", &vec![], &Scope::new());
        assert!(result.is_ok());
        let current_dir = env::current_dir().unwrap().to_str().unwrap().to_string();
        assert_eq!(current_dir, current_dir);
    }
}
