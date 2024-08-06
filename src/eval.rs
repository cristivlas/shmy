use crate::cmds::{get_command, Exec, RegisteredCommand};
use gag::Redirect;
use glob::glob;
use regex;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::env;
use std::fmt::{self, Debug};
use std::iter::Peekable;
use std::process::{Command as StdCommand, Stdio};
use std::rc::Rc;
use std::str::FromStr;

#[macro_export]
macro_rules! debug_print {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            dbg!($($arg)*)
        } else {
            ($($arg)*)
        }
    };
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    And,
    Assign,
    Div,
    Equals,
    Gt,
    Gte,
    IntDiv,
    Minus,
    Mod,
    Mul,
    Lt,
    Lte,
    NotEquals,
    Or,
    Pipe,
    Plus,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Op::And => write!(f, "&&"),
            Op::Assign => write!(f, "="),
            Op::Div => write!(f, "/"),
            Op::Equals => write!(f, "=="),
            Op::Gt => write!(f, ">"),
            Op::Gte => write!(f, ">="),
            Op::IntDiv => write!(f, "//"),
            Op::Minus => write!(f, "-"),
            Op::Mod => write!(f, "%"),
            Op::Mul => write!(f, "*"),
            Op::Lt => write!(f, "<"),
            Op::Lte => write!(f, "<="),
            Op::NotEquals => write!(f, "!="),
            Op::Or => write!(f, "||"),
            Op::Pipe => write!(f, "|"),
            Op::Plus => write!(f, "+"),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Priority {
    Low,
    High,
}

impl Op {
    fn priority(&self) -> Priority {
        match &self {
            Op::Assign
            | Op::Gt
            | Op::Gte
            | Op::Lt
            | Op::Lte
            | Op::NotEquals
            | Op::Minus
            | Op::Pipe
            | Op::Plus => Priority::Low,
            _ => Priority::High,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    End,
    Literal(String),
    Operator(Op),
    LeftParen,
    RightParen,
    Semicolon,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Location {
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}:{}]", self.line, self.col)
    }
}

/// Trait for objects with location info.
trait HasLocation {
    fn loc(&self) -> Location;
}

impl Location {
    fn new() -> Self {
        Self { line: 1, col: 0 }
    }

    fn next_line(&mut self) {
        self.line += 1;
        self.col = 0;
    }
}

macro_rules! derive_has_location {
    ($type:ty) => {
        impl HasLocation for $type {
            fn loc(&self) -> Location {
                self.loc
            }
        }
    };
}

#[derive(Debug)]
pub struct EvalError {
    loc: Location,
    message: String,
}

impl EvalError {
    pub fn show(&self, input: &String) {
        let line = self.loc.line as usize;
        let col = self.loc.col as usize;

        // Get the problematic line from the input
        let lines: Vec<&str> = input.lines().collect();
        let error_line = lines.get(line - 1).unwrap_or(&"");

        // Create the error indicator
        let indicator = "-".repeat(col) + "^";

        eprintln!("Error at line {}, column {}:", line, col + 1);
        eprintln!("{}", error_line);
        eprintln!("{}", indicator);
        eprintln!("{}", self.message);
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.loc, self.message)
    }
}

pub type EvalResult<T = ()> = std::result::Result<T, EvalError>;

fn error<T: HasLocation, R>(w: &T, message: &str) -> EvalResult<R> {
    Err(EvalError {
        loc: w.loc(),
        message: message.to_string(),
    })
}

/// Non-terminal AST node.
trait ExprNode {
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult;
}

struct Parser<I: Iterator<Item = char>> {
    chars: Peekable<I>,
    loc: Location,
    comment: bool,
    escaped: bool,
    in_quotes: bool,
    expect_else_expr: bool,
    empty: Rc<Expression>,
    current_expr: Rc<Expression>,
    scope: Rc<Scope>,
    expr_stack: Vec<Rc<Expression>>,
    scope_stack: Vec<Rc<Scope>>,
    group: Rc<Expression>,
    group_stack: Vec<Rc<Expression>>,
    globbed_tokens: Vec<String>,
}

impl<I: Iterator<Item = char>> HasLocation for Parser<I> {
    fn loc(&self) -> Location {
        self.loc
    }
}

/// Tokenizer helper.
macro_rules! token {
    // Single character token
    ($self:expr, $tok:expr, $single_token:expr) => {{
        $self.next();
        $tok = $single_token;
    }};

    // Double character token only
    ($self:expr, $tok:expr, $second:expr, $double_token:expr) => {{
        $self.next();
        if let Some(&next_c) = $self.chars.peek() {
            if next_c == $second {
                $self.next();
                $tok = $double_token;
                continue;
            }
        }
    }};

    // Single or double character token
    ($self:expr, $tok:expr,$second:expr, $single_token:expr, $double_token:expr) => {{
        $self.next();
        if let Some(&next_c) = $self.chars.peek() {
            if next_c == $second {
                $self.next();
                $tok = $double_token;
                continue;
            }
        }
        $tok = $single_token;
    }};
}

impl<T> Parser<T>
where
    T: Iterator<Item = char>,
{
    fn new(input: T, scope: &Rc<Scope>) -> Self {
        let empty = Rc::new(Expression::Empty);
        let loc = Location::new();
        Self {
            chars: input.peekable(),
            loc,
            comment: false,
            escaped: false,
            in_quotes: false,
            expect_else_expr: false,
            empty: Rc::clone(&empty),
            current_expr: Rc::clone(&empty),
            scope: Rc::clone(&scope),
            expr_stack: Vec::new(),
            scope_stack: Vec::new(),
            group: new_group(loc),
            group_stack: Vec::new(),
            globbed_tokens: Vec::new(),
        }
    }

    fn empty(&self) -> Rc<Expression> {
        Rc::clone(&self.empty)
    }

    fn is_delimiter(&self, tok: &str, c: char) -> bool {
        // Forward slashes and dashes need special handling, since they occur in
        // paths and command line options; it is unreasonable to require quotes.
        if "'/-'".contains(c) {
            if tok.is_empty() {
                return !self.group.is_args()
                    && !self.current_expr.is_cmd()
                    && !self.current_expr.is_empty();
            }
            match parse_value(tok, self.loc, &self.scope) {
                Ok(Value::Int(_)) | Ok(Value::Real(_)) => true,
                _ => false,
            }
        } else {
            const DELIMITERS: &str = " \t\n\r()+=;|&<>!";
            DELIMITERS.contains(c)
        }
    }

    fn next(&mut self) {
        self.loc.col += 1;
        self.chars.next();
    }

    fn glob_literal(&mut self, literal: String, quoted: bool) -> EvalResult<Token> {
        // This function should not be called if globbed_tokens are not depleted.
        assert!(self.globbed_tokens.is_empty());

        if !quoted {
            match glob(&literal) {
                Ok(paths) => {
                    self.globbed_tokens = paths
                        .filter_map(Result::ok)
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect();

                    if !self.globbed_tokens.is_empty() {
                        return Ok(Token::Literal(self.globbed_tokens.remove(0)));
                    }
                }
                Err(_) => {} // Ignore glob errors and treat as literal
            }
        }

        Ok(Token::Literal(literal))
    }

    #[rustfmt::skip]
    pub fn next_token(&mut self) -> EvalResult<Token> {

        if !self.globbed_tokens.is_empty() {
            return Ok(Token::Literal(self.globbed_tokens.remove(0)));
        }

        let mut tok = Token::End;
        let mut literal = String::new();
        let mut quoted = false;

        while let Some(c) = self.chars.peek().cloned() {
            if tok != Token::End {
                break;
            }

            if c == '\n' {
                self.loc.next_line();
                self.comment = false;
                self.next();
                continue;
            }
            if self.comment {
                self.next();
                continue;
            }
            match c {
                '#' => { self.comment = true; self.next(); }
                '%' => token!(self, tok, Token::Operator(Op::Mod)),
                '(' => token!(self, tok, Token::LeftParen),
                ')' => token!(self, tok, Token::RightParen),
                ';' => token!(self, tok, Token::Semicolon),
                '+' => token!(self, tok, Token::Operator(Op::Plus)),

                // Give glob precedence over multiplication. For multiplication, use \*
                '\\' => token!(self, tok, '*', Token::Operator(Op::Mul)),

                '&' => token!(self, tok, '&', Token::Operator(Op::And)),
                '|' => token!(self, tok, '|', Token::Operator(Op::Pipe), Token::Operator(Op::Or)),
                '!' => token!(self, tok, '=', Token::Operator(Op::NotEquals)),
                '<' => token!(self, tok, '=', Token::Operator(Op::Lt), Token::Operator(Op::Lte)),
                '>' => token!(self, tok, '=', Token::Operator(Op::Gt), Token::Operator(Op::Gte)),
                '=' => token!(self, tok, '=', Token::Operator(Op::Assign), Token::Operator(Op::Equals)),
                '-' => { if !self.is_delimiter(&literal, c) {
                        literal.push(c);
                    } else {
                        tok = Token::Operator(Op::Minus);
                    }
                    self.next();
                }
                '/' => if !self.is_delimiter(&literal, c) {
                    // Treat forward slashes as chars in arguments to commands, to avoid quoting file paths.
                        literal.push(c);
                        self.next();
                    } else {
                        token!(self, tok, '/', Token::Operator(Op::Div), Token::Operator(Op::IntDiv));
                }
                _ => {
                    if c.is_whitespace() {
                        self.next();
                        if !literal.is_empty() {
                            break;
                        }
                        continue;
                    }

                    while let Some(&next_c) = self.chars.peek() {
                        if self.escaped {
                            match next_c {
                                'n' => literal.push('\n'),
                                't' => literal.push('\t'),
                                'r' => literal.push('\r'),
                                _ => literal.push(next_c),
                            }
                            self.next();
                            self.escaped = false;
                        } else if next_c == '\\' {
                            self.next();
                            if self.in_quotes {
                                // Escapes only work inside quotes, to avoid
                                // issues with path delimiters under Windows
                                self.escaped = true;
                            } else {
                                literal.push('\\');
                            }
                        } else if next_c == '"' {
                            quoted = true;
                            self.in_quotes ^= true;
                            self.next();
                        } else {
                            if self.in_quotes || !self.is_delimiter(&literal, next_c) {
                                literal.push(next_c);
                                self.next();
                            } else {
                                break;
                            }
                        }
                    }

                    if !literal.is_empty() {
                        assert!(literal != "-" && literal != "/");

                        tok = self.glob_literal(literal.clone(), quoted)?;
                        literal.clear();
                    }
                }
            }
        }
        if self.in_quotes {
            return error(self, "Unbalanced quotes");
        }

        // Check for partial token, handle special cases such as single fwd slash.
        if tok == Token::End && !literal.is_empty() {
            if literal == "-" && self.current_expr.is_number() {
                tok = Token::Operator(Op::Minus);
            } else if literal == "/" && self.current_expr.is_number() {
                tok = Token::Operator(Op::Div);
            } else {
                tok = self.glob_literal(literal.clone(), quoted)?;
            }
        }

        Ok(tok)
    }

    /// Add an expression to the AST.
    fn add_expr(&mut self, expr: &Rc<Expression>) -> EvalResult {
        assert!(!expr.is_empty());

        if self.expect_else_expr {
            self.current_expr = self.expr_stack.pop().unwrap();
            self.expect_else_expr = false;
        }

        let ref current = *self.current_expr;
        match current {
            Expression::Args(e) => e.borrow_mut().add_child(expr),
            Expression::Bin(e) => e.borrow_mut().add_child(expr),
            Expression::Branch(e) => e.borrow_mut().add_child(expr),
            Expression::Cmd(e) => e.borrow_mut().add_child(expr),
            Expression::Empty => {
                self.current_expr = Rc::clone(expr);
                Ok(())
            }
            Expression::Group(e) => e.borrow_mut().add_child(expr),
            Expression::For(e) => {
                e.borrow_mut().add_child(expr)?;
                if !e.borrow().body.is_empty() {
                    self.clear_current();
                }
                Ok(())
            }
            Expression::Lit(_) => {
                if let Expression::Args(a) = &*self.group {
                    a.borrow_mut().group.push(Rc::clone(&self.current_expr));
                    self.current_expr = Rc::clone(&expr);
                    Ok(())
                } else {
                    error(self, "Dangling expression after literal")
                }
            }
            Expression::Loop(e) => e.borrow_mut().add_child(expr),
        }
    }

    fn pop_binary_ops(&mut self, statement: bool) -> EvalResult {
        while let Some(stack_top) = self.expr_stack.last() {
            // If the expression on the top of the expression stack is a binary
            // expression, pop it, make it the new current expression, and add
            // old current as a child.
            // If this operation does not occur at the end of a statement, do
            // not pop the stack past assignments.
            if stack_top.is_bin_op(statement) {
                let expr = Rc::clone(&self.current_expr);
                self.current_expr = self.expr_stack.pop().unwrap();

                if !expr.is_empty() {
                    self.add_expr(&expr)?;
                }
            } else {
                break;
            }
        }
        Ok(())
    }

    fn add_current_expr_to_group(&mut self) -> EvalResult {
        let group = Rc::clone(&self.group);

        if let Expression::Args(g) = &*group {
            self.pop_binary_ops(true)?;
            if !self.current_expr.is_empty() {
                g.borrow_mut().group.push(Rc::clone(&self.current_expr));
            }
            self.pop_group()?;
        } else if !self.current_expr.is_empty() {
            if let Expression::Group(g) = &*group {
                self.pop_binary_ops(true)?;
                g.borrow_mut().group.push(Rc::clone(&self.current_expr));
            } else {
                panic!("Unexpected group error");
            }
        }
        Ok(())
    }

    fn clear_current(&mut self) {
        self.current_expr = self.empty();
    }

    fn finalize_groups(&mut self) -> EvalResult {
        if self.group.is_args() {
            self.add_current_expr_to_group()?; // Finalize pending cmd line args
        }
        self.add_current_expr_to_group()
    }

    fn push(&mut self, group: Group) -> EvalResult {
        if group != Group::None {
            // Save the current scope
            let current_scope = Rc::clone(&self.scope);
            self.scope_stack.push(current_scope.clone());
            // Create new scope and make it current
            self.scope = Scope::new(Some(current_scope));
            // Start a new group
            self.group_stack.push(Rc::clone(&self.group));

            if group == Group::Args {
                self.group = new_args(self.loc);
            } else {
                self.group = new_group(self.loc);
            }
        }
        self.expr_stack.push(Rc::clone(&self.current_expr));
        self.clear_current();

        Ok(())
    }

    fn pop(&mut self) -> EvalResult {
        self.finalize_groups()?;
        self.pop_group()
    }

    fn pop_group(&mut self) -> EvalResult {
        if !self.expr_stack.is_empty() {
            self.current_expr = self.expr_stack.pop().unwrap();
            self.add_expr(&Rc::clone(&self.group))?;
        }

        self.group = self.group_stack.pop().unwrap(); // Restore group
        self.scope = self.scope_stack.pop().unwrap(); // Restore scope

        Ok(())
    }

    fn parse(&mut self, quit: &mut bool) -> EvalResult<Rc<Expression>> {
        loop {
            let tok = self.next_token()?;
            match &tok {
                Token::End => {
                    break;
                }
                Token::LeftParen => {
                    self.push(Group::Explicit)?;
                }
                Token::RightParen => {
                    if self.group_stack.is_empty() {
                        return error(self, "Unmatched right parenthesis");
                    }
                    self.pop()?;
                }
                Token::Semicolon => {
                    self.finalize_groups()?;
                    if !self.current_expr.is_for() {
                        self.clear_current();
                    }
                }
                Token::Literal(ref s) => {
                    // keywords
                    let word = s.to_lowercase();
                    if ["exit", "quit"].iter().any(|&cmd| cmd == word) {
                        *quit = true;
                        break;
                    }
                    if word == "if" {
                        let expr = Rc::new(Expression::Branch(RefCell::new(BranchExpr {
                            cond: self.empty(),
                            if_branch: self.empty(),
                            else_branch: self.empty(),
                            expect_else: false, // becomes true once "else" keyword is seen
                            loc: self.loc,
                        })));
                        self.add_expr(&expr)?;
                    } else if word == "in" {
                        if let Expression::For(f) = &*self.current_expr {
                            if f.borrow().var.is_empty() {
                                return error(self, "Expecting identifier in FOR expression");
                            }
                        } else {
                            return error(self, "IN without FOR");
                        }
                        self.push(Group::Args)?; // args will be added to ForExpr when finalized
                    } else if word == "else" {
                        if let Expression::Branch(b) = &*self.current_expr {
                            if !b.borrow_mut().is_else_expected() {
                                return error(self, "Conditional expression or IF branch missing");
                            }
                            self.expect_else_expr = true;
                            self.push(Group::None)?;
                        } else {
                            return error(self, "ELSE without IF");
                        }
                    } else if word == "for" {
                        let expr = Rc::new(Expression::For(RefCell::new(ForExpr {
                            var: String::default(),
                            args: self.empty(),
                            body: self.empty(),
                            scope: Rc::clone(&self.scope),
                            loc: self.loc,
                        })));
                        self.add_expr(&expr)?;
                        self.current_expr = expr;
                    } else if word == "while" {
                        let expr = Rc::new(Expression::Loop(RefCell::new(LoopExpr {
                            cond: self.empty(),
                            body: self.empty(),
                            loc: self.loc,
                        })));
                        self.add_expr(&expr)?;
                    } else if let Some(cmd) = if !self.group.is_args() {
                        get_command(s)
                    } else {
                        None
                    } {
                        // Commands
                        let expr = Rc::new(Expression::Cmd(RefCell::new(Command {
                            cmd,
                            args: self.empty(),
                            loc: self.loc,
                            scope: Rc::clone(&self.scope),
                        })));
                        self.add_expr(&expr)?;
                        self.current_expr = expr;
                        self.push(Group::Args)?; // args will be added to command when finalized
                    } else {
                        // Identifiers and literals
                        let expr = Rc::new(Expression::Lit(Rc::new(Literal {
                            tok,
                            loc: self.loc,
                            scope: Rc::clone(&self.scope),
                        })));
                        self.add_expr(&expr)?;
                    }
                }
                Token::Operator(op) => {
                    if *op == Op::Pipe && self.group.is_args() {
                        // Finish the arguments of the left hand-side command.
                        self.add_current_expr_to_group()?;
                    }

                    if op.priority() == Priority::Low {
                        self.pop_binary_ops(false)?;
                    }

                    let expr = Rc::new(Expression::Bin(RefCell::new(BinExpr {
                        op: op.clone(),
                        lhs: Rc::clone(&self.current_expr),
                        rhs: self.empty(),
                        loc: self.loc,
                        scope: Rc::clone(&self.scope),
                    })));

                    if op.priority() == Priority::Low {
                        self.expr_stack.push(Rc::clone(&expr));
                        self.clear_current();
                    } else {
                        self.current_expr = expr;
                    }
                }
            }
        }

        self.finalize_groups()?;

        if !self.expr_stack.is_empty() {
            let msg = if self.expect_else_expr {
                "Dangling ELSE"
            } else {
                dbg!(&self.expr_stack);
                "Unbalanced parenthesis"
            };
            return error(self, msg);
        }
        assert!(self.group_stack.is_empty()); // because the expr_stack is empty

        Ok(Rc::clone(&self.group))
    }
}

#[derive(Clone, Debug)]
pub struct Variable {
    val: Rc<RefCell<Value>>,
}

impl Variable {
    fn new(val: Value) -> Self {
        Self {
            val: Rc::new(RefCell::new(val)),
        }
    }

    fn assign(&self, val: Value) {
        *self.val.borrow_mut() = val;
    }

    pub fn value(&self) -> Value {
        self.val.borrow().clone()
    }
}

impl From<&str> for Variable {
    fn from(value: &str) -> Self {
        Variable {
            val: Rc::new(RefCell::new(value.parse::<Value>().unwrap())),
        }
    }
}

impl fmt::Display for Variable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self.val.borrow())
    }
}

// #[derive(Debug)]
pub struct Scope {
    parent: Option<Rc<Scope>>,
    pub vars: RefCell<HashMap<String, Variable>>,
}

impl Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parent: {:p}", &self.parent)
    }
}

impl Scope {
    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Self {
            parent: parent,
            vars: RefCell::new(HashMap::new()),
        })
    }

    fn new_from_env() -> Rc<Scope> {
        let vars = env::vars()
            .map(|(key, value)| (key, Variable::from(value.as_str())))
            .collect::<HashMap<_, _>>();

        Rc::new(Scope {
            parent: None,
            vars: RefCell::new(vars),
        })
    }

    fn insert(&self, name: String, val: Value) {
        self.vars.borrow_mut().insert(name, Variable::new(val));
    }

    pub fn lookup(&self, s: &str) -> Option<Variable> {
        match self.vars.borrow().get(s) {
            Some(v) => Some(v.clone()),
            None => match &self.parent {
                Some(scope) => scope.lookup(s),
                _ => None,
            },
        }
    }

    pub fn lookup_value(&self, s: &str) -> Option<Value> {
        match self.lookup(s) {
            Some(v) => Some(v.value()),
            None => None,
        }
    }
}

fn parse_value(s: &str, loc: Location, scope: &Rc<Scope>) -> EvalResult<Value> {
    match regex::Regex::new(r"\$\{?([a-zA-Z_][a-zA-Z0-9_]*)\}?") {
        Ok(regex) => {
            let result = regex.replace_all(s, |caps: &regex::Captures| {
                let var_name = &caps[1];
                match scope.lookup(var_name) {
                    Some(var) => var.value().to_string(),
                    None => caps
                        .get(0)
                        .map_or(String::new(), |m| m.as_str().to_string()), // Leave unchanged
                }
            });
            result.parse::<Value>()
        }
        Err(e) => Err(EvalError {
            loc: loc,
            message: e.to_string(),
        }),
    }
}

#[derive(Debug)]
enum Expression {
    Empty,
    Args(RefCell<GroupExpr>),
    Bin(RefCell<BinExpr>),
    Cmd(RefCell<Command>),
    Branch(RefCell<BranchExpr>),
    For(RefCell<ForExpr>),
    Group(RefCell<GroupExpr>),
    Lit(Rc<Literal>),
    Loop(RefCell<LoopExpr>),
}

impl Expression {
    fn expand_args(&self, scope: &Rc<Scope>) -> EvalResult<Vec<String>> {
        let mut args = Vec::new();

        if let Expression::Args(a) = &self {
            for arg in &a.borrow().group {
                let v = arg.eval()?;
                let mut s = v.to_string();
                // Expand leading tilde.
                if s.starts_with('~') {
                    match scope.lookup("HOME") {
                        Some(v) => {
                            s = format!("{}{}", v.value().to_string(), &s[1..]);
                        }
                        _ => {}
                    }
                }
                args.push(s);
            }
        } else if !self.is_empty() {
            return error(self, "Expression is not an argument list");
        }

        Ok(args)
    }

    fn expand_arg_values(&self) -> EvalResult<Vec<Value>> {
        let mut vals: Vec<Value> = Vec::new();

        if let Expression::Args(a) = &self {
            for arg in &a.borrow().group {
                vals.push(arg.eval()?);
            }
        } else if !self.is_empty() {
            return error(self, "Expression is not an argument list");
        }

        Ok(vals)
    }

    fn is_args(&self) -> bool {
        matches!(self, Expression::Args(_))
    }

    fn is_bin_op(&self, sequence: bool) -> bool {
        if let Expression::Bin(b) = &self {
            sequence || b.borrow().op != Op::Assign
        } else {
            false
        }
    }

    fn is_cmd(&self) -> bool {
        matches!(self, Expression::Cmd(_))
    }

    fn is_for(&self) -> bool {
        matches!(self, Expression::For(_))
    }

    fn is_empty(&self) -> bool {
        matches!(self, Expression::Empty)
    }

    fn is_group(&self) -> bool {
        matches!(self, Expression::Group(_))
    }

    fn is_number(&self) -> bool {
        if self.is_empty() {
            return false;
        }
        match self.eval() {
            Ok(Value::Int(_)) | Ok(Value::Real(_)) => true,
            _ => false,
        }
    }
}

impl fmt::Display for Expression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Expression::Args(group) => write!(f, "{}", group.borrow()),
            Expression::Bin(bin_expr) => write!(f, "{}", bin_expr.borrow()),
            Expression::Cmd(cmd) => write!(f, "{}", cmd.borrow()),
            Expression::Branch(branch) => write!(f, "{}", branch.borrow()),
            Expression::Group(group) => write!(f, "{}", group.borrow()),
            Expression::For(for_expr) => write!(f, "{}", for_expr.borrow()),
            Expression::Empty => write!(f, ""),
            Expression::Lit(literal) => write!(f, "{}", literal),
            Expression::Loop(loop_expr) => write!(f, "{}", loop_expr.borrow()),
        }
    }
}

impl HasLocation for Expression {
    fn loc(&self) -> Location {
        match self {
            Expression::Args(group) => group.borrow().loc(),
            Expression::Bin(bin_expr) => bin_expr.borrow().loc(),
            Expression::Cmd(cmd) => cmd.borrow().loc(),
            Expression::Branch(branch) => branch.borrow().loc(),
            Expression::Group(group) => group.borrow().loc(),
            Expression::For(for_expr) => for_expr.borrow().loc(),
            Expression::Empty => panic!("Empty expression"),
            Expression::Lit(literal) => literal.loc(),
            Expression::Loop(loop_expr) => loop_expr.borrow().loc(),
        }
    }
}

#[derive(Debug)]
struct BinExpr {
    op: Op,
    lhs: Rc<Expression>,
    rhs: Rc<Expression>,
    loc: Location,
    scope: Rc<Scope>, // Scope needed for assignment op.
}

derive_has_location!(BinExpr);

impl ExprNode for BinExpr {
    /// Add right hand-side child expression.
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult {
        if self.rhs.is_empty() {
            self.rhs = Rc::clone(child);
            Ok(())
        } else {
            dbg!(&self.rhs, &child);
            error(self, "Dangling expression")
        }
    }
}

impl fmt::Display for BinExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {} {}", self.lhs, self.op, self.rhs)
    }
}

/// Division evaluator helper
macro_rules! div_match {
    ($self:expr, $i:expr, $rhs:expr) => {
        match $rhs {
            Value::Int(j) => {
                if j == 0 {
                    error($self, "Division by zero")
                } else {
                    Ok(Value::Real(($i as f64) / (j as f64)))
                }
            }
            Value::Real(j) => {
                if j == 0.0 {
                    error($self, "Division by zero")
                } else {
                    Ok(Value::Real(($i as f64) / j))
                }
            }
            // Value::Str(_) => error($self, "Cannot divide number by string"),
            Value::Str(s) => Ok(Value::Str(format!("{}/{}", $i, s))),
        }
    };
}

/// Macro to generate comparison functions
macro_rules! eval_cmp_fn {
    ($fn_name:ident, $op:tt) => {
        fn $fn_name(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
            match self.eval_cmp(lhs, rhs)? {
                Value::Real(r) => Ok(Value::Int((r $op 0.0) as i64)),
                _ => panic!("Unexpected result type in comparison"),
            }
        }
    }
}

impl BinExpr {
    fn eval_and(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        Ok(Value::Int((value_as_bool(lhs) && value_as_bool(rhs)) as _))
    }

    fn eval_assign(&self, rhs: Value) -> EvalResult<Value> {
        if let Expression::Lit(lit) = &*self.lhs {
            if let Token::Literal(name) = &lit.tok {
                if name.starts_with('$') {
                    if let Some(var) = lit.scope.lookup(&name[1..]) {
                        var.assign(rhs);
                        return Ok(var.value());
                    } else {
                        return error(self, &format!("Variable not found: {}", name));
                    }
                } else {
                    self.scope.insert(name.to_owned(), rhs.clone());
                    return Ok(rhs);
                }
            }
        }
        error(self, "Identifier expected on left hand-side of assignment")
    }

    fn eval_cmp(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => Ok(Value::Real((i - j) as _)),
                Value::Real(j) => Ok(Value::Real(i as f64 - j)),
                Value::Str(_) => error(self, "Cannot compare number to string"),
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => Ok(Value::Real(i - j as f64)),
                Value::Real(j) => Ok(Value::Real(i - j)),
                Value::Str(_) => error(self, "Cannot compare number to string"),
            },
            Value::Str(s1) => match rhs {
                Value::Int(_) | Value::Real(_) => error(self, "Cannot compare string to number"),
                Value::Str(s2) => {
                    let ord = match s1.cmp(&s2) {
                        Ordering::Equal => 0,
                        Ordering::Less => -1,
                        Ordering::Greater => 1,
                    };
                    Ok(Value::Real(ord as _))
                }
            },
        }
    }

    eval_cmp_fn!(eval_equals, ==);
    eval_cmp_fn!(eval_not_equals, !=);
    eval_cmp_fn!(eval_lt, <);
    eval_cmp_fn!(eval_lte, <=);
    eval_cmp_fn!(eval_gt, >);
    eval_cmp_fn!(eval_gte, >=);

    fn eval_div(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        match lhs {
            Value::Int(i) => div_match!(self, i, rhs),
            Value::Real(i) => div_match!(self, i, rhs),
            Value::Str(s1) => match rhs {
                // Value::Int(_) | Value::Real(_) => error(self, "Cannot divide string by number"),
                Value::Int(_) | Value::Real(_) => Ok(Value::Str(format!("{}/{}", s1, rhs))),
                Value::Str(s2) => Ok(Value::Str(format!("{}/{}", s1, s2))),
            },
        }
    }

    fn eval_int_div(&self, _lhs: Value, _rhs: Value) -> EvalResult<Value> {
        todo!()
    }

    fn eval_minus(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => Ok(Value::Int(i - j)),
                Value::Real(j) => Ok(Value::Real(i as f64 - j)),
                Value::Str(_) => error(self, "Cannot subtract string from number"),
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => Ok(Value::Real(i - j as f64)),
                Value::Real(j) => Ok(Value::Real(i - j)),
                Value::Str(_) => error(self, "Cannot subtract string from number"),
            },
            Value::Str(_) => match rhs {
                Value::Int(_) | Value::Real(_) => error(self, "Cannot subtract number from string"),
                Value::Str(_) => error(self, "Cannot subtract strings"),
            },
        }
    }

    fn eval_mod(&self, _lhs: Value, _rhs: Value) -> EvalResult<Value> {
        todo!()
    }

    fn eval_mul(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => Ok(Value::Int(i * j)),
                Value::Real(j) => Ok(Value::Real(i as f64 * j)),
                Value::Str(_) => error(self, "Cannot multiply number by string"),
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => Ok(Value::Real(i * j as f64)),
                Value::Real(j) => Ok(Value::Real(i * j)),
                Value::Str(_) => error(self, "Cannot multiply number by string"),
            },
            Value::Str(_) => match rhs {
                Value::Int(_) | Value::Real(_) => error(self, "Cannot multiply string by number"),
                Value::Str(_) => error(self, "Cannot multiply strings"),
            },
        }
    }

    fn eval_or(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        Ok(Value::Int((value_as_bool(lhs) || value_as_bool(rhs)) as _))
    }

    fn eval_pipe(&self, lhs: &Rc<Expression>, rhs: &Rc<Expression>) -> EvalResult<Value> {
        if lhs.is_empty() {
            return error(self, "Expecting pipe input");
        }
        // Create a pipe
        let (reader, writer) = match os_pipe::pipe() {
            Ok((r, w)) => (r, w),
            Err(e) => return error(self, &format!("Failed to create pipe: {}", e)),
        };

        // Redirect stdout to the pipe
        let redirect = match Redirect::stdout(writer) {
            Ok(r) => r,
            Err(e) => return error(self, &format!("Failed to redirect stdout: {}", e)),
        };

        // Get our own program name
        let program = match env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                return error(self, &format!("Failed to get executable name: {}", e));
            }
        };

        // Get the right-hand side expression as a string
        let rhs_str = rhs.to_string();

        debug_print!(&program, &rhs_str);

        // Start a copy of the running program with the arguments "-c" rhs_str
        let child = match StdCommand::new(&program)
            .arg("-c")
            .arg(&rhs_str)
            .stdin(Stdio::from(reader))
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return error(self, &format!("Failed to spawn child process: {}", e)),
        };

        // Left-side evaluation's stdout goes into the pipe.
        let lhs_result = lhs.eval();

        // Drop the redirect to close the write end of the pipe
        drop(redirect);

        // Wait for the child process to complete and get its output
        let output = match child.wait_with_output() {
            Ok(o) => o,
            Err(e) => return error(self, &format!("Failed to get child process output: {}", e)),
        };
        lhs_result?;
        print!("{}", String::from_utf8_lossy(&output.stdout));
        Ok(Value::Int(output.status.code().unwrap_or_else(|| -1) as _))
    }

    fn eval_plus(&self, lhs: Value, rhs: Value) -> EvalResult<Value> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => Ok(Value::Int(i + j)),
                Value::Real(j) => Ok(Value::Real(i as f64 + j)),
                Value::Str(ref s) => Ok(Value::Str(format!("{}{}", i, s))),
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => Ok(Value::Real(i + j as f64)),
                Value::Real(j) => Ok(Value::Real(i + j)),
                Value::Str(ref s) => Ok(Value::Str(format!("{}{}", i, s))),
            },
            Value::Str(s) => Ok(Value::Str(format!("{}{}", s, rhs))),
        }
    }
}

macro_rules! eval_bin {
    ($self:expr, $f:ident) => {
        $self.$f($self.lhs.eval()?, $self.rhs.eval()?)
    };
}

impl Eval for BinExpr {
    fn eval(&self) -> EvalResult<Value> {
        if self.rhs.is_empty() {
            error(self, "Expecting right hand-side expression")
        } else if self.lhs.is_empty() {
            eval_unary(self, &self.op, self.rhs.eval()?)
        } else {
            match self.op {
                Op::And => eval_bin!(self, eval_and),
                Op::Assign => self.eval_assign(self.rhs.eval()?.clone()),
                Op::Div => eval_bin!(self, eval_div),
                Op::Gt => eval_bin!(self, eval_gt),
                Op::Gte => eval_bin!(self, eval_gte),
                Op::IntDiv => eval_bin!(self, eval_int_div),
                Op::Equals => eval_bin!(self, eval_equals),
                Op::Lt => eval_bin!(self, eval_lt),
                Op::Lte => eval_bin!(self, eval_lte),
                Op::Minus => eval_bin!(self, eval_minus),
                Op::Mod => eval_bin!(self, eval_mod),
                Op::Mul => eval_bin!(self, eval_mul),
                Op::NotEquals => eval_bin!(self, eval_not_equals),
                Op::Or => eval_bin!(self, eval_or),
                Op::Pipe => self.eval_pipe(&self.lhs, &self.rhs),
                Op::Plus => eval_bin!(self, eval_plus),
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Group {
    None,
    Args,
    Explicit,
}

#[derive(Debug)]
struct GroupExpr {
    kind: Group,
    group: Vec<Rc<Expression>>,
    loc: Location,
}

impl GroupExpr {
    fn new_args(loc: Location) -> Self {
        Self {
            kind: Group::Args,
            group: Vec::new(),
            loc,
        }
    }

    fn new_group(loc: Location) -> Self {
        Self {
            kind: Group::Explicit,
            group: Vec::new(),
            loc,
        }
    }
}

derive_has_location!(GroupExpr);

impl Eval for GroupExpr {
    fn eval(&self) -> EvalResult<Value> {
        let mut result = Ok(Value::Int(0));

        for e in &self.group {
            result = e.eval();
            if result.is_err() {
                break;
            }
        }
        result // return the last evaluation
    }
}

impl ExprNode for GroupExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult {
        self.group.push(Rc::clone(child));
        Ok(())
    }
}

fn join_expr(expressions: &[Rc<Expression>], separator: &str) -> String {
    expressions
        .iter()
        .map(|expr| expr.to_string())
        .collect::<Vec<_>>()
        .join(separator)
}

impl fmt::Display for GroupExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == Group::Args {
            write!(f, "{}", join_expr(&self.group, " "))
        } else {
            write!(f, "( {} )", join_expr(&self.group, "; "))
        }
    }
}

#[derive(Debug)]
struct Command {
    cmd: RegisteredCommand,
    args: Rc<Expression>,
    loc: Location,
    scope: Rc<Scope>,
}

derive_has_location!(Command);

impl Eval for Command {
    fn eval(&self) -> EvalResult<Value> {
        let args = self.args.expand_args(&self.scope)?;
        self.cmd
            .exec(&self.cmd.name(), &args, &self.scope)
            .map_err(|e| EvalError {
                loc: self.loc,
                message: e,
            })
    }
}

impl ExprNode for Command {
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult {
        assert!(child.is_args());
        assert!(self.args.is_empty());
        self.args = Rc::clone(&child);
        Ok(())
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.cmd.name(), self.args)
    }
}

#[derive(Debug)]
struct BranchExpr {
    cond: Rc<Expression>,
    if_branch: Rc<Expression>,
    else_branch: Rc<Expression>,
    expect_else: bool,
    loc: Location,
}

derive_has_location!(BranchExpr);

impl BranchExpr {
    fn is_else_expected(&mut self) -> bool {
        if !self.cond.is_empty() && !self.if_branch.is_empty() {
            self.expect_else = true;
            return true;
        }
        false
    }
}

fn value_as_bool(val: Value) -> bool {
    match val {
        Value::Int(i) => i != 0,
        Value::Real(r) => r != 0.0,
        Value::Str(s) => !s.is_empty(),
    }
}

fn eval_as_bool(expr: &Rc<Expression>) -> EvalResult<bool> {
    Ok(value_as_bool(expr.eval()?))
}

impl ExprNode for BranchExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult {
        if self.cond.is_empty() {
            self.cond = Rc::clone(child);
        } else if self.if_branch.is_empty() {
            if !child.is_group() {
                return error(self, "IF branch must be enclosed in parenthesis");
            }
            self.if_branch = Rc::clone(child);
        } else if self.else_branch.is_empty() {
            if !self.expect_else {
                return error(self, "Expecting ELSE keyword");
            }
            if !child.is_group() {
                return error(&**child, "ELSE branch must be enclosed in parenthesis");
            }
            self.else_branch = Rc::clone(child);
        } else {
            return error(self, "Dangling expression after else branch");
        }
        Ok(())
    }
}

impl Eval for BranchExpr {
    fn eval(&self) -> EvalResult<Value> {
        if self.cond.is_empty() {
            return error(self, "Expecting IF condition");
        } else if self.if_branch.is_empty() {
            return error(self, "Expecting IF branch");
        }
        if eval_as_bool(&self.cond)? {
            self.if_branch.eval()
        } else if self.else_branch.is_empty() {
            Ok(Value::Int(0))
        } else {
            self.else_branch.eval()
        }
    }
}

impl fmt::Display for BranchExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "if {} {}", self.cond, self.if_branch)?;

        if !self.else_branch.is_empty() {
            write!(f, " else {}", self.else_branch)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Literal {
    tok: Token,
    loc: Location,
    scope: Rc<Scope>,
}

derive_has_location!(Literal);

impl Eval for Literal {
    fn eval(&self) -> EvalResult<Value> {
        match &self.tok {
            Token::Literal(s) => parse_value(&s, self.loc, &self.scope),
            _ => {
                panic!("Invalid token type in literal expression");
            }
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.tok {
            Token::LeftParen => write!(f, "("),
            Token::RightParen => write!(f, ")"),
            Token::Semicolon => write!(f, ";"),
            Token::Literal(s) => write!(f, "\"{}\"", &s),
            Token::Operator(op) => write!(f, "{}", op),
            Token::End => write!(f, ""),
        }
    }
}

#[derive(Debug)]
struct LoopExpr {
    cond: Rc<Expression>,
    body: Rc<Expression>,
    loc: Location,
}

derive_has_location!(LoopExpr);

impl Eval for LoopExpr {
    fn eval(&self) -> EvalResult<Value> {
        if self.cond.is_empty() {
            return error(self, "Expecting WHILE condition");
        } else if self.body.is_empty() {
            return error(self, "Expecting WHILE body");
        }
        let mut result = Ok(Value::Int(0));
        loop {
            if !eval_as_bool(&self.cond)? {
                break;
            }
            result = self.body.eval();

            if result.is_err() {
                break;
            }
        }
        result
    }
}

impl ExprNode for LoopExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult {
        if self.cond.is_empty() {
            self.cond = Rc::clone(child);
        } else if self.body.is_empty() {
            if !child.is_group() {
                return error(&**child, "WHILE body must be enclosed in parenthesis");
            }
            self.body = Rc::clone(&child);
        } else {
            return error(self, "WHILE already has a body");
        }
        Ok(())
    }
}

impl fmt::Display for LoopExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "while ({}) {}", self.cond, self.body)
    }
}

#[derive(Debug)]
struct ForExpr {
    var: String,
    args: Rc<Expression>,
    body: Rc<Expression>,
    scope: Rc<Scope>,
    loc: Location,
}

derive_has_location!(ForExpr);

impl Eval for ForExpr {
    fn eval(&self) -> EvalResult<Value> {
        if self.var.is_empty() {
            return error(self, "Expecting FOR variable");
        }
        if self.args.is_empty() {
            return error(self, "Expecting argument list");
        }
        if self.body.is_empty() {
            return error(self, "Expecting FOR body");
        }
        let vals = self.args.expand_arg_values()?;
        let mut result = Value::Int(0);

        for v in &vals {
            self.scope.insert(self.var.clone(), v.clone());
            result = self.body.eval()?;
        }
        Ok(result)
    }
}

impl ExprNode for ForExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> EvalResult {
        if self.var.is_empty() {
            if let Expression::Lit(lit) = &**child {
                if let Token::Literal(name) = &lit.tok {
                    self.var = name.clone();
                    return Ok(());
                }
            }
            return error(self, "Expecting identifier FOR expression");
        } else if self.args.is_empty() {
            if child.is_args() {
                self.args = Rc::clone(&child);
            } else {
                return error(self, "Expecting argument list");
            }
        } else if self.body.is_empty() {
            if !child.is_group() {
                return error(self, "FOR body must be enclosed in parenthesis");
            }
            self.body = Rc::clone(&child);
        } else {
            return error(self, "FOR already has a body");
        }
        Ok(())
    }
}

impl fmt::Display for ForExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "for {} in {} ({})", &self.var, self.args, self.body)
    }
}

fn eval_unary<T: HasLocation>(loc: &T, op: &Op, val: Value) -> EvalResult<Value> {
    match op {
        Op::Minus => match val {
            Value::Int(i) => Ok(Value::Int(-i)),
            Value::Real(r) => Ok(Value::Real(-r)),
            Value::Str(s) => Ok(Value::Str(format!("-{}", s))),
        },
        _ => error(loc, "Expecting left-hand term in binary operation"),
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Int(i64),
    Real(f64),
    Str(String),
}

impl Default for Value {
    fn default() -> Self {
        Value::Str(String::default())
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self {
            Value::Int(v) => {
                write!(f, "{}", v)
            }
            Value::Real(v) => {
                write!(f, "{}", v)
            }
            Value::Str(s) => {
                write!(f, "{}", s)
            }
        }
    }
}

impl FromStr for Value {
    type Err = EvalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Ok(i) = s.parse::<i64>() {
            Ok(Value::Int(i))
        } else if let Ok(f) = s.parse::<f64>() {
            Ok(Value::Real(f))
        } else {
            Ok(Value::Str(s.to_string()))
        }
    }
}

trait Eval {
    fn eval(&self) -> EvalResult<Value>;
}

impl Eval for Expression {
    fn eval(&self) -> EvalResult<Value> {
        match &self {
            Expression::Args(g) => g.borrow().eval(),
            Expression::Bin(b) => b.borrow().eval(),
            Expression::Branch(b) => b.borrow().eval(),
            Expression::Cmd(c) => c.borrow().eval(),
            Expression::Group(g) => g.borrow().eval(),
            Expression::For(f) => f.borrow().eval(),
            Expression::Empty => {
                panic!("Empty expression");
            }
            Expression::Lit(lit) => lit.eval(),
            Expression::Loop(l) => l.borrow().eval(),
        }
    }
}

pub struct Interp {
    scope: Rc<Scope>,
}

fn new_args(loc: Location) -> Rc<Expression> {
    Rc::new(Expression::Args(RefCell::new(GroupExpr::new_args(loc))))
}

fn new_group(loc: Location) -> Rc<Expression> {
    Rc::new(Expression::Group(RefCell::new(GroupExpr::new_group(loc))))
}

impl Interp {
    pub fn new() -> Self {
        Self {
            scope: Scope::new_from_env(),
        }
    }

    pub fn eval(&mut self, quit: &mut bool, input: &str) -> EvalResult<Value> {
        debug_print!(input);
        let ast = self.parse(quit, input)?;

        debug_print!(&ast);
        ast.eval()
    }

    fn parse(&mut self, quit: &mut bool, input: &str) -> EvalResult<Rc<Expression>> {
        let mut parser = Parser::new(input.chars(), &self.scope);
        parser.parse(quit)
    }

    pub fn set_var(&mut self, name: &str, value: String) {
        self.scope.insert(name.to_string(), Value::Str(value))
    }

    pub fn get_scope(&self) -> Rc<Scope> {
        Rc::clone(&self.scope)
    }
}
