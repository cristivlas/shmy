use crate::{eval::Value, utils::executable};
use colored::*;
use std::cell::{Ref, RefCell, RefMut};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::env;
use std::fmt::{self, Debug};
use std::hash::{Hash, Hasher};
use std::io::IsTerminal;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug)]
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

/// An abstraction for representing an identifier (name).
#[derive(Debug, Clone)]
pub struct Ident(Arc<String>);

/// Environmental variables are case-insensitive (but case-preserving) in Windows,
/// therefore all shell variable lookups should have consistent behavior.
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
        Self(Arc::new(value.to_string()))
    }
}

impl From<String> for Ident {
    fn from(value: String) -> Self {
        Ident::from(value.as_str())
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

pub trait Namespace {
    fn clear(&self);
    fn keys<F: Fn(&Ident) -> bool>(&self, pred: F) -> Vec<String>;
    fn insert(&self, ident: &Ident, val: Value) -> Option<Variable>;
    fn lookup(&self, ident: &Ident) -> Option<Ref<Variable>>;
    fn remove(&self, ident: &Ident) -> Option<Variable>;
}

struct VarTable {
    vars: RefCell<HashMap<Ident, Variable>>,
}

impl VarTable {
    fn new() -> Self {
        Self {
            vars: RefCell::new(HashMap::new()),
        }
    }

    fn with_vars(vars: HashMap<Ident, Variable>) -> Self {
        Self {
            vars: RefCell::new(vars),
        }
    }

    fn inner(&self) -> Ref<HashMap<Ident, Variable>> {
        Ref::map(self.vars.borrow(), |vars| vars)
    }

    fn inner_mut(&self) -> RefMut<HashMap<Ident, Variable>> {
        RefMut::map(self.vars.borrow_mut(), |vars| vars)
    }
}

impl Namespace for VarTable {
    fn clear(&self) {
        self.vars.borrow_mut().clear();
    }

    /// Filter keys (identifiers) by predicate
    fn keys<F: Fn(&Ident) -> bool>(&self, pred: F) -> Vec<String> {
        self.vars
            .borrow()
            .iter()
            .filter_map(|(k, _)| {
                if pred(k) {
                    Some(k.view().to_string())
                } else {
                    None
                }
            })
            .collect()
    }

    fn insert(&self, ident: &Ident, val: Value) -> Option<Variable> {
        self.vars
            .borrow_mut()
            .insert(ident.clone(), Variable::new(val))
    }

    fn lookup(&self, ident: &Ident) -> Option<Ref<Variable>> {
        Ref::filter_map(self.vars.borrow(), |vars| vars.get(ident)).ok()
    }

    fn remove(&self, ident: &Ident) -> Option<Variable> {
        self.vars.borrow_mut().remove(ident)
    }
}

pub struct Scope {
    pub parent: Option<Arc<Scope>>,
    vars: VarTable,
    err_arg: RefCell<usize>, // Index of argument with error.
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
    pub fn new() -> Arc<Scope> {
        Arc::new(Self {
            parent: None,
            vars: VarTable::new(),
            err_arg: RefCell::default(),
        })
    }

    pub fn with_parent(parent: Option<Arc<Scope>>) -> Arc<Scope> {
        Arc::new(Self {
            parent,
            vars: VarTable::new(),
            err_arg: RefCell::default(),
        })
    }

    pub fn with_env_vars() -> Arc<Scope> {
        env::set_var("SHELL", executable().unwrap_or("shmy".to_string()));

        let vars: HashMap<Ident, Variable> = env::vars()
            .map(|(key, value)| (Ident::from(key), Variable::from(value.as_str())))
            .collect::<HashMap<_, _>>();

        Arc::new(Scope {
            parent: None,
            vars: VarTable::with_vars(vars),
            err_arg: RefCell::default(),
        })
    }

    pub fn is_interrupted() -> bool {
        crate::INTERRUPT_EVENT
            .try_lock()
            .and_then(|guard| Ok(guard.is_set()))
            .unwrap_or(false)
    }

    pub fn clear(&self) {
        self.vars.clear();
        *self.err_arg.borrow_mut() = 0;
    }

    pub fn insert(&self, name: String, val: Value) {
        self.vars.insert(&Ident::from(name), val);
    }

    pub fn insert_value(&self, name: &Arc<String>, val: Value) {
        self.vars.insert(&Ident(Arc::clone(name)), val);
    }

    pub fn lookup(&self, name: &str) -> Option<Ref<Variable>> {
        self.lookup_by_ident(&Ident::from(name))
    }

    fn lookup_by_ident(&self, ident: &Ident) -> Option<Ref<Variable>> {
        self.vars.lookup(ident).or_else(|| {
            self.parent
                .as_ref()
                .and_then(|scope| scope.lookup_by_ident(ident))
        })
    }

    pub fn lookup_local(&self, name: &str) -> Option<Ref<Variable>> {
        self.vars.lookup(&Ident::from(name))
    }

    pub fn lookup_starting_with(&self, name: &str) -> Vec<String> {
        let ident = Ident::from(name);
        self.vars.keys(|k| k.view().starts_with(&ident.view()))
    }

    pub fn lookup_value(&self, name: &str) -> Option<Value> {
        self.lookup(name).map(|v| v.value().clone())
    }

    /// Lookup and erase a variable
    fn erase_by_ident(&self, ident: &Ident) -> Option<Variable> {
        if self.parent.is_none() {
            // The top-most scope (global scope) shadows the environment
            env::remove_var(ident.view());
        }
        self.vars
            .remove(ident)
            .or_else(|| self.parent.as_ref().and_then(|p| p.erase_by_ident(ident)))
    }

    pub fn erase(&self, name: &str) -> Option<Variable> {
        self.erase_by_ident(&Ident::from(name))
    }

    /// Return the global scope
    pub fn global<'a>(&'a self) -> &'a Scope {
        if self.parent.is_none() {
            &self
        } else {
            let mut current = self.parent.as_ref().unwrap();
            while let Some(parent) = &current.parent {
                current = &parent;
            }
            &*current
        }
    }

    pub fn vars(&self) -> Ref<HashMap<Ident, Variable>> {
        self.vars.inner()
    }

    pub fn vars_mut(&self) -> RefMut<HashMap<Ident, Variable>> {
        self.vars.inner_mut()
    }

    /// Getter and setter for the index of the argument that caused an error.
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

    /// Colorize string shown in errors and warnings.
    pub fn err_str(&self, path: &str) -> ColoredString {
        self.color(&path, Color::BrightCyan, &std::io::stderr())
    }

    /// Colorize the error and set the index of the argument that caused the error
    pub fn err_path_arg(&self, path: &str, args: &[String]) -> ColoredString {
        self.set_err_arg(args.iter().position(|a| a == path).unwrap_or(0));
        self.err_str(path)
    }

    pub fn err_path(&self, path: &Path) -> ColoredString {
        // TOOD: Canonicalize the path here?
        self.err_str(&path.display().to_string())
    }

    /// Show Ctrl-Z / Ctrl-D hint.
    /// For situations where user input is expected. Examples
    /// ```
    /// cat
    /// ```
    /// for i in -; (ls $i)
    /// ```
    pub fn show_eof_hint(&self) {
        if std::io::stdin().is_terminal() {
            #[cfg(windows)]
            const MESSAGE: &str = "Press Ctrl-Z to end input";
            #[cfg(not(windows))]
            const MESSAGE: &str = "Press Ctrl-D to end input";

            eprintln!(
                "{}",
                self.color(MESSAGE, Color::BrightCyan, &std::io::stderr())
            );
        }
    }
}
