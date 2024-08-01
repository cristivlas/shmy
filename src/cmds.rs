use crate::eval::Value;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::process::Command;
use std::rc::Rc;
use std::sync::Mutex;
use which::which;
mod echo;
mod ls;

pub trait Exec {
    fn exec(&self, args: Vec<String>) -> Result<Value, String>;
}

#[derive(Clone)]
pub struct BuiltinCommand {
    name: String,
    exec: Rc<dyn Exec>,
}

impl Exec for BuiltinCommand {
    fn exec(&self, args: Vec<String>) -> Result<Value, String> {
        self.exec.exec(args)
    }
}

unsafe impl Send for BuiltinCommand {}

lazy_static! {
    pub static ref COMMAND_REGISTRY: Mutex<HashMap<String, BuiltinCommand>> =
        Mutex::new(HashMap::new());
}

pub fn register_command(command: BuiltinCommand) {
    COMMAND_REGISTRY
        .lock()
        .unwrap()
        .insert(command.name.clone(), command);
}

pub fn get_command(name: &str) -> Option<BuiltinCommand> {
    let mut cmd = COMMAND_REGISTRY.lock().unwrap().get(name).cloned();
    if cmd.is_none() {
        if let Some(path) = locate_executable(name) {
            register_command(BuiltinCommand {
                name: name.to_string(),
                exec: Rc::new(External { path }),
            });
            cmd = COMMAND_REGISTRY.lock().unwrap().get(name).cloned();
        }
    }
    cmd
}

fn locate_executable(name: &str) -> Option<String> {
    match which(name) {
        Ok(path) => Some(path.to_string_lossy().to_string()),
        Err(_) => None,
    }
}

struct External {
    path: String,
}

impl Exec for External {
    fn exec(&self, args: Vec<String>) -> Result<crate::eval::Value, String> {
        // Print the command being executed (for debugging purposes)
        // println!("{} {}", self.path, args.join(" "));

        // Execute the external program.
        // TODO: customize environment vars.
        let mut command = Command::new(&self.path);
        command.args(&args);
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
}
