use crate::eval::Scope;
use std::rc::Rc;

/// Copy variables from the current scope outwards into the environment of the
/// command to be executed, but do not carry over special redirect variables.
pub fn copy_vars_to_command_env(command: &mut std::process::Command, scope: &Rc<Scope>) {
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
