use crate::eval::Value;
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
mod echo;
mod ls;

pub trait Exec {
    fn exec(&self, args: Vec<String>) -> Result<Value, String>;
}

#[derive(Clone)]
pub struct BuiltinCommand {
    name: &'static str,
    exec: Rc<dyn Exec>,
}

impl Exec for BuiltinCommand {
    fn exec(&self, args: Vec<String>) -> Result<Value, String> {
        self.exec.exec(args)
    }
}

unsafe impl Send for BuiltinCommand {}

lazy_static! {
    pub static ref COMMAND_REGISTRY: Mutex<HashMap<&'static str, BuiltinCommand>> =
        Mutex::new(HashMap::new());
}

pub fn register_command(command: BuiltinCommand) {
    COMMAND_REGISTRY
        .lock()
        .unwrap()
        .insert(command.name, command);
}

pub fn get_command(name: &str) -> Option<BuiltinCommand> {
    COMMAND_REGISTRY.lock().unwrap().get(name).cloned()
}
