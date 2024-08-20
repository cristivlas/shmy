use crate::eval::Scope;
use std::env;
use std::rc::Rc;

/// Copy variables from the current scope outwards into the environment of the
/// command to be executed, but do not carry over special redirect variables.
pub(crate) fn copy_vars_to_command_env(command: &mut std::process::Command, scope: &Rc<Scope>) {
    // Override existing environment variables
    command.env_clear();

    let mut current_scope = scope;
    loop {
        for (key, variable) in current_scope.vars.borrow().iter() {
            if key != "__stdout" && key != "__stderr" {
                command.env(key, variable.value().to_string());
            }
        }
        // Walk up the enclosing scope
        match &current_scope.parent {
            None => {
                break;
            }
            Some(scope) => {
                current_scope = scope;
            }
        }
    }
}

pub(crate) fn get_own_path() -> Result<String, String> {
    match env::current_exe() {
        Ok(p) => {
            #[cfg(test)]
            {
                use regex::Regex;

                let path_str = p.to_string_lossy();
                #[cfg(windows)]
                {
                    let re = Regex::new(r"\\deps\\.*?(\..*)?$").map_err(|e| e.to_string())?;
                    Ok(re.replace(&path_str, "\\mysh$1").to_string())
                }
                #[cfg(not(windows))]
                {
                    let re = Regex::new(r"/deps/.+?(\..*)?$").map_err(|e| e.to_string())?;
                    Ok(re.replace(&path_str, "/mysh$1").to_string())
                }
            }
            #[cfg(not(test))]
            {
                Ok(p.to_string_lossy().to_string())
            }
        }
        Err(e) => Err(format!("Failed to get executable name: {}", e)),
    }
}
