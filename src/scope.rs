use crate::eval::Value;
use crate::utils::executable;
use colored::*;
use std::cell::{Ref, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::env;
use std::fmt::{self, Debug};
use std::hash::{Hash, Hasher};
use std::io::IsTerminal;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::Ordering::SeqCst;

#[derive(Clone, Debug, PartialEq)]
pub struct Variable {
    val: RefCell<Value>,
}

impl Variable {
    pub fn new(val: Value) -> Self {
        Self {
            val: RefCell::new(val),
        }
    }

    pub fn assign(&self, val: Value) -> Ref<Value> {
        *self.val.borrow_mut() = val;
        self.val.borrow()
    }

    pub fn value(&self) -> Ref<Value> {
        Ref::map(self.val.borrow(), |v| v)
    }
}

impl From<&str> for Variable {
    fn from(value: &str) -> Self {
        Variable {
            val: RefCell::new(value.parse::<Value>().unwrap()),
        }
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.val.borrow())
    }
}

#[derive(Debug, Clone)]
pub struct Ident(String);

#[cfg(windows)]
impl Ident {
    pub fn view(&self) -> String {
        self.0.to_uppercase()
    }
}

#[cfg(not(windows))]
impl Ident {
    pub fn view(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Ident {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl PartialEq for Ident {
    fn eq(&self, other: &Self) -> bool {
        self.view() == other.view()
    }
}

impl Eq for Ident {}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        self.view().cmp(&other.view())
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for Ident {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.view().hash(state);
    }
}

impl fmt::Display for Ident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Ident {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_special_var(&self) -> bool {
        matches!(self.as_str(), "__errors" | "__stderr" | "__stdout")
    }
}

#[derive(PartialEq)]
pub struct Scope {
    pub parent: Option<Rc<Scope>>,
    pub vars: RefCell<HashMap<Ident, Variable>>,
    err_arg: RefCell<usize>, // TODO: pass &mut Rc<Scope> to Exec::exec
}

impl Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parent_addr: Option<String> = self.parent.as_ref().map(|p| format!("{:p}", p));

        f.debug_struct("Scope")
            .field("address", &format_args!("{:p}", self))
            .field("parent", &parent_addr)
            .finish()
    }
}

impl Scope {
    pub fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Self {
            parent,
            vars: RefCell::new(HashMap::new()),
            err_arg: RefCell::default(),
        })
    }

    pub fn with_env_vars() -> Rc<Scope> {
        env::set_var("SHELL", executable().unwrap_or("mysh".to_string()));

        let vars: HashMap<Ident, Variable> = env::vars()
            .map(|(key, value)| (Ident(key), Variable::from(value.as_str())))
            .collect::<HashMap<_, _>>();

        Rc::new(Scope {
            parent: None,
            vars: RefCell::new(vars),
            err_arg: RefCell::default(),
        })
    }

    pub fn is_interrupted(&self) -> bool {
        crate::INTERRUPT.load(SeqCst)
    }

    pub fn clear(&self) {
        self.vars.borrow_mut().clear();
        *self.err_arg.borrow_mut() = 0;
    }

    pub fn insert(&self, name: String, val: Value) {
        self.vars
            .borrow_mut()
            .insert(Ident(name), Variable::new(val));
    }

    pub fn lookup(&self, name: &str) -> Option<Ref<Variable>> {
        self.lookup_by_ident(&Ident::from(name))
    }

    fn lookup_by_ident(&self, ident: &Ident) -> Option<Ref<Variable>> {
        Ref::filter_map(self.vars.borrow(), |vars| vars.get(ident))
            .ok()
            .or_else(|| {
                self.parent
                    .as_ref()
                    .and_then(|scope| scope.lookup_by_ident(ident))
            })
    }

    pub fn lookup_local(&self, name: &str) -> Option<Ref<Variable>> {
        Ref::filter_map(self.vars.borrow(), |vars| vars.get(&Ident::from(name))).ok()
    }

    pub fn lookup_starting_with(&self, name: &str) -> Vec<String> {
        let var_name = Ident::from(name);
        let mut keys = Vec::new();

        for key in self.vars.borrow().keys() {
            if key.view().starts_with(&var_name.view()) {
                keys.push(key.0.clone())
            }
        }
        keys
    }

    pub fn lookup_value(&self, name: &str) -> Option<Value> {
        self.lookup(name).map(|v| v.value().clone())
    }

    /// Lookup and erase a variable
    fn erase_by_ident(&self, name: &Ident) -> Option<Variable> {
        match self.vars.borrow_mut().remove(name) {
            Some(var) => Some(var),
            None => match &self.parent {
                Some(scope) => scope.erase_by_ident(name),
                _ => None,
            },
        }
    }

    pub fn erase(&self, name: &str) -> Option<Variable> {
        self.erase_by_ident(&Ident::from(name))
    }

    /// Return the global scope
    pub fn global(&self) -> Rc<Scope> {
        let mut current = self.parent.as_ref().unwrap();
        while let Some(parent) = &current.parent {
            current = &parent;
        }
        Rc::clone(&current)
    }

    pub fn err_arg(&self) -> usize {
        *self.err_arg.borrow()
    }

    pub fn set_err_arg(&self, index: usize) {
        *self.err_arg.borrow_mut() = index + 1;
    }

    /// The evaluation scope is passed to commands via the Exec trait;
    /// this is a convenient place to check for NO_COLOR.
    /// TODO: CLICOLOR, CLICOLOR_FORCE? See: https://bixense.com/clicolors/
    pub fn use_colors<T: IsTerminal>(&self, out: &T) -> bool {
        self.lookup("NO_COLOR").is_none() && out.is_terminal()
    }

    pub fn color<T: IsTerminal>(&self, t: &str, c: Color, out: &T) -> ColoredString {
        if self.use_colors(out) {
            t.color(c)
        } else {
            t.normal()
        }
    }

    /// Colorize paths shown in errors and warnings.
    pub fn err_path_str(&self, path: &str) -> ColoredString {
        self.color(
            &path,
            Color::TrueColor {
                r: 255,
                g: 165,
                b: 0,
            },
            &std::io::stderr(),
        )
    }

    pub fn err_path(&self, path: &Path) -> ColoredString {
        self.err_path_str(&path.display().to_string())
    }
}
