use crate::eval::Scope;
use std::rc::Rc;

pub fn copy_vars_to_command_env(command: &mut std::process::Command, scope: &Rc<Scope>) {
    // Override existing environment variables
    command.env_clear();

    let mut current_scope = scope;
    loop {
        for (key, variable) in current_scope.vars.borrow().iter() {
            command.env(key, variable.value().to_string());
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
