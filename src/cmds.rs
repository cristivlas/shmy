use crate::eval::{Scope, Value};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::fmt::Debug;
use std::process::Command;
use std::rc::Rc;
use std::sync::Mutex;
use which::which;
mod cat;
mod cd;
mod clear;
mod echo;
mod ls;
mod vars;

mod flags;

pub trait Exec {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String>;
    fn is_external(&self) -> bool;
}

#[derive(Clone)]
pub struct RegisteredCommand {
    name: String,
    inner: Rc<dyn Exec>,
}

impl RegisteredCommand {
    pub fn name(&self) -> &String {
        &self.name
    }
}

impl Debug for RegisteredCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "name: {}", &self.name)
    }
}

impl Exec for RegisteredCommand {
    fn exec(&self, name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        self.inner.exec(name, args, scope)
    }
    fn is_external(&self) -> bool {
        self.inner.is_external()
    }
}

unsafe impl Send for RegisteredCommand {}

lazy_static! {
    pub static ref COMMAND_REGISTRY: Mutex<HashMap<String, RegisteredCommand>> =
        Mutex::new(HashMap::new());
}

pub fn register_command(command: RegisteredCommand) {
    COMMAND_REGISTRY
        .lock()
        .unwrap()
        .insert(command.name.clone(), command);
}

pub fn get_command(name: &str) -> Option<RegisteredCommand> {
    let mut cmd = COMMAND_REGISTRY.lock().unwrap().get(name).cloned();
    if cmd.is_none() {
        if let Some(path) = locate_executable(name) {
            register_command(RegisteredCommand {
                name: name.to_string(),
                inner: Rc::new(External { path }),
            });
            cmd = COMMAND_REGISTRY.lock().unwrap().get(name).cloned();
        }
    }
    cmd
}

pub fn list_registered_commands() -> Vec<String> {
    let registry = COMMAND_REGISTRY.lock().unwrap();
    registry.keys().cloned().collect()
}

fn locate_executable(name: &str) -> Option<String> {
    match which(name) {
        Ok(path) => Some(path.to_string_lossy().to_string()),
        Err(_) => None,
    }
}

// Wrap execution of an external program.
struct External {
    path: String,
}

impl Exec for External {
    fn exec(&self, _name: &str, args: &Vec<String>, scope: &Rc<Scope>) -> Result<Value, String> {
        let mut command = Command::new(&self.path);
        command.args(args);

        // Clear existing environment variables
        command.env_clear();

        // Set environment variables from the scope
        for (key, variable) in scope.vars.borrow().iter() {
            command.env(key, variable.value().to_string());
        }

        match command.spawn() {
            Ok(mut child) => match child.wait() {
                Ok(status) => {
                    if let Some(code) = status.code() {
                        Ok(Value::Int(code as _))
                    } else {
                        Ok(Value::Str("".to_owned()))
                    }
                }
                Err(e) => Err(format!("Failed to wait on child process: {}", e)),
            },
            Err(e) => Err(format!("Failed to execute command: {}", e)),
        }
    }

    fn is_external(&self) -> bool {
        true
    }
}
