use crate::cmds::{get_command, Exec};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::iter::Peekable;
use std::rc::Rc;
use std::{fmt, process};

macro_rules! debug_dbg {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            dbg!($($arg)*)
        } else {
            ($($arg)*)
        }
    };
}

fn is_reserved(c: char) -> bool {
    const RESERVED_CHARS: &str = " \t\n\r()+-=;/*";
    RESERVED_CHARS.contains(c)
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Assign,
    Equals,
    Minus,
    Plus,
    Div,
    Mul,
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
struct Location {
    line: u32,
    col: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}:{}]", self.line, self.col)
    }
}

/// Trait for objects with location info.
trait HasLocation {
    fn loc(&self) -> &Location;
}

macro_rules! derive_has_location {
    ($type:ty) => {
        impl HasLocation for $type {
            fn loc(&self) -> &Location {
                &self.loc
            }
        }
    };
}

fn error<T: HasLocation, R>(t: &T, s: &str) -> Result<R, String> {
    return Err(format!("{} {}", t.loc(), s));
}

/// Non-terminal AST node.
trait ExprNode {
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<bool, String>;
}

struct Parser<I: Iterator<Item = char>> {
    chars: Peekable<I>,
    loc: Location,
    escaped: bool,
    quoted: bool,
    expect_else_expr: bool,
    empty: Rc<Expression>,
    current_expr: Rc<Expression>,
    scope: Rc<Scope>,
    expr_stack: Vec<Rc<Expression>>,
    scope_stack: Vec<Rc<Scope>>,
    group: Rc<Expression>,
    group_stack: Vec<Rc<Expression>>,
}

impl<I: Iterator<Item = char>> HasLocation for Parser<I> {
    fn loc(&self) -> &Location {
        &self.loc
    }
}

impl<T> Parser<T>
where
    T: Iterator<Item = char>,
{
    fn empty(&self) -> Rc<Expression> {
        Rc::clone(&self.empty)
    }

    fn next(&mut self) {
        self.loc.col += 1;
        self.chars.next();
    }

    pub fn next_token(&mut self) -> Result<Token, String> {
        let mut tok = Token::End;
        let mut dashes = String::new();

        while let Some(c) = self.chars.peek() {
            if tok != Token::End {
                break;
            }
            match c {
                '(' => {
                    self.next();
                    tok = Token::LeftParen;
                }
                ')' => {
                    self.next();
                    tok = Token::RightParen;
                }
                ';' => {
                    self.next();
                    tok = Token::Semicolon;
                }
                '+' => {
                    self.next();
                    tok = Token::Operator(Op::Plus);
                }
                '-' => {
                    self.next();
                    if self.is_arg_expected() {
                        dashes.push('-');
                    } else {
                        tok = Token::Operator(Op::Minus);
                    }
                }
                '/' => {
                    self.next();
                    tok = Token::Operator(Op::Div);
                }
                '*' => {
                    self.next();
                    tok = Token::Operator(Op::Mul);
                }
                '=' => {
                    self.next();
                    if let Some(&next_c) = self.chars.peek() {
                        if next_c == '=' {
                            tok = Token::Operator(Op::Equals);
                            self.next();
                            continue;
                        }
                    }
                    tok = Token::Operator(Op::Assign);
                }
                '\n' => {
                    self.loc.line += 1;
                    self.loc.col = 1;
                    self.next();
                }
                _ => {
                    if c.is_whitespace() {
                        self.next();
                        if !dashes.is_empty() {
                            tok = Token::Literal(dashes);
                            dashes = String::new();
                        }
                        continue;
                    }

                    let mut has_chars = false;
                    let mut literal = String::new();

                    if !dashes.is_empty() {
                        (literal, dashes) = (dashes, literal);
                    }
                    while let Some(&next_c) = self.chars.peek() {
                        if next_c == '\\' {
                            if self.escaped {
                                literal.push(next_c);
                            }
                            self.next();
                            self.escaped ^= true;
                            continue;
                        }
                        if next_c == '"' {
                            if !self.escaped {
                                self.quoted ^= true;
                                self.next();
                                continue;
                            }
                        }
                        has_chars = true;
                        if self.quoted || self.escaped || !is_reserved(next_c) {
                            literal.push(next_c);
                            self.next();
                        } else {
                            break;
                        }
                        self.escaped = false;
                    }
                    if has_chars && literal.is_empty() {
                        error(self, "Empty token")?;
                    } else {
                        tok = Token::Literal(literal);
                    }
                }
            }
        }
        if self.quoted {
            error(self, "Unbalanced quotes")?;
        }
        Ok(tok)
    }

    /// Add an expression to the AST.
    fn add_expr(&mut self, expr: &Rc<Expression>) -> Result<(), String> {
        if expr.is_empty() {
            error(self, "Unexpected empty expression")?;
        }

        if self.expect_else_expr {
            self.current_expr = self.expr_stack.pop().unwrap();
            self.expect_else_expr = false;
        }

        let ref current = *self.current_expr;
        match current {
            Expression::Bin(e) => e.borrow_mut().add_child(expr)?,
            Expression::Branch(e) => e.borrow_mut().add_child(expr)?,
            Expression::Cmd(e) => e.borrow_mut().add_child(expr)?,
            Expression::Empty => {
                self.current_expr = Rc::clone(expr);
                true
            }
            Expression::Group(e) => e.borrow_mut().add_child(expr)?,
            Expression::Lit(_, _) => error(self, "Dangling expression after literal")?,
            Expression::Loop(e) => e.borrow_mut().add_child(expr)?,
        };

        Ok(())
    }

    fn add_current_expr_to_group(&mut self) {
        if !self.current_expr.is_empty() {
            if let Expression::Group(g) = &*self.group {
                g.borrow_mut().group.push(Rc::clone(&self.current_expr));
            } else {
                panic!("Unexpected group error");
            }
            self.current_expr = self.empty(); // Clear the current expression
        }
    }

    fn finalize_group(&mut self) {
        self.add_current_expr_to_group();
    }

    fn is_arg_expected(&self) -> bool {
        let ref current = *self.current_expr;
        matches!(current, Expression::Cmd(_))
    }

    fn push(&mut self, make_group: bool) {
        if make_group {
            // Save the current scope
            let current_scope = Rc::clone(&self.scope);
            self.scope_stack.push(current_scope.clone());
            // Create new scope and make it current
            self.scope = Scope::new(Some(current_scope));
            // Start a new group
            self.group_stack.push(Rc::clone(&self.group));
            self.group = new_group(self.loc);
        }

        // Push current expression, and make the empty expression current
        self.expr_stack.push(Rc::clone(&self.current_expr));
        self.current_expr = self.empty();
    }

    fn pop(&mut self) -> Result<(), String> {
        self.finalize_group();
        assert!(self.current_expr.is_empty());

        self.current_expr = self.expr_stack.pop().unwrap();

        let expr = {
            if let Expression::Group(g) = &*self.group {
                let group = &g.borrow().group;
                if group.len() == 1 {
                    Rc::clone(&group[0])
                } else {
                    Rc::clone(&self.group)
                }
            } else {
                panic!("Unexpected group error");
            }
        };

        self.add_expr(&expr)?;

        self.group = self.group_stack.pop().unwrap(); // Restore group
        self.scope = self.scope_stack.pop().unwrap(); // Restore scope

        Ok(())
    }
}

#[derive(Clone, Debug)]
struct Variable {
    val: Rc<RefCell<Value>>,
}

impl Variable {
    fn assign(&self, val: Value) {
        *self.val.borrow_mut() = val;
    }

    fn new(val: Value) -> Variable {
        Variable {
            val: Rc::new(RefCell::new(val)),
        }
    }

    fn value(&self) -> Value {
        self.val.borrow().clone()
    }
}

#[derive(Debug)]
struct Scope {
    parent: Option<Rc<Scope>>,
    vars: RefCell<HashMap<String, Variable>>,
}

impl Scope {
    fn insert(&self, name: String, val: Value) {
        self.vars.borrow_mut().insert(name, Variable::new(val));
    }

    fn lookup(&self, s: &str) -> Option<Variable> {
        match self.vars.borrow().get(s) {
            Some(v) => Some(v.clone()),
            None => match &self.parent {
                Some(scope) => scope.lookup(s),
                _ => None,
            },
        }
    }

    fn new(parent: Option<Rc<Scope>>) -> Rc<Scope> {
        Rc::new(Scope {
            parent: parent,
            vars: RefCell::new(HashMap::new()),
        })
    }
}

#[derive(Debug)]
enum Expression {
    Empty,
    Bin(RefCell<BinExpr>),
    Cmd(RefCell<Command>),
    Branch(RefCell<BranchExpr>),
    Group(RefCell<GroupExpr>),
    Lit(Token, Rc<Scope>),
    Loop(RefCell<LoopExpr>),
}

impl Expression {
    fn is_group(&self) -> bool {
        matches!(self, Expression::Group(_))
    }
    fn is_empty(&self) -> bool {
        matches!(self, Expression::Empty)
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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<bool, String> {
        assert!(!self.lhs.is_empty());

        if self.rhs.is_empty() {
            self.rhs = Rc::clone(child);
            Ok(true)
        } else {
            error(self, "Dangling expression")
        }
    }
}

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
            Value::Str(_) => error($self, "Cannot divide number by string"),
        }
    };
}

impl BinExpr {
    fn eval_assign(&self, rhs: Value) -> Result<Value, String> {
        if let Expression::Lit(tok, scope) = &*self.lhs {
            if let Token::Literal(name) = tok {
                if name.starts_with('$') {
                    if let Some(var) = scope.lookup(&name[1..]) {
                        var.assign(rhs);
                        return Ok(var.value());
                    } else {
                        error(self, "Variable not found")?;
                    }
                } else {
                    self.scope.insert(name.to_owned(), rhs.clone());
                    return Ok(rhs);
                }
            }
        }
        error(self, "Identifier expected on left hand-side of assignment")
    }

    fn eval_cmp(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => Ok(Value::Int(i - j)),
                Value::Real(j) => Ok(Value::Int((i as f64 - j) as i64)),
                Value::Str(_) => error(self, "Cannot compare number to string"),
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => Ok(Value::Int((i - j as f64) as i64)),
                Value::Real(j) => Ok(Value::Int((i - j) as i64)),
                Value::Str(_) => error(self, "Cannot compare number to string"),
            },
            Value::Str(s1) => match rhs {
                Value::Int(_) | Value::Real(_) => error(self, "Cannot comapre string to number"),
                Value::Str(s2) => {
                    let ord = match s1.cmp(&s2) {
                        Ordering::Equal => 0,
                        Ordering::Less => -1,
                        Ordering::Greater => 1,
                    };
                    Ok(Value::Int(ord))
                }
            },
        }
    }

    fn eval_div(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
        match lhs {
            Value::Int(i) => div_match!(self, i, rhs),
            Value::Real(i) => div_match!(self, i, rhs),
            Value::Str(_) => match rhs {
                Value::Int(_) | Value::Real(_) => error(self, "Cannot divide string by number"),
                Value::Str(_) => error(self, "Cannot divide strings"),
            },
        }
    }

    fn eval_minus(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
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

    fn eval_mul(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
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

    fn eval_plus(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
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
            Value::Str(i) => {
                let format_string = |j: &dyn fmt::Display| Ok(Value::Str(format!("{}{}", i, j)));

                match rhs {
                    Value::Int(j) => format_string(&j),
                    Value::Real(j) => format_string(&j),
                    Value::Str(ref j) => format_string(j),
                }
            }
        }
    }
}

impl Eval for BinExpr {
    fn eval(&self) -> Result<Value, String> {
        assert!(!self.lhs.is_empty());

        if self.rhs.is_empty() {
            error(self, "Expecting right hand-side expression")?;
        }

        let rhs = self.rhs.eval()?;

        match self.op {
            Op::Assign => self.eval_assign(rhs.clone()),
            Op::Equals => match self.eval_cmp(self.lhs.eval()?, rhs)? {
                Value::Int(i) => Ok(Value::Int((i == 0) as i64)),
                _ => panic!("Unexpected non-integer result"),
            },
            Op::Minus => self.eval_minus(self.lhs.eval()?, rhs),
            Op::Plus => self.eval_plus(self.lhs.eval()?, rhs),
            Op::Div => self.eval_div(self.lhs.eval()?, rhs),
            Op::Mul => self.eval_mul(self.lhs.eval()?, rhs),
        }
    }
}

#[derive(Debug)]
struct GroupExpr {
    group: Vec<Rc<Expression>>,
    loc: Location,
}

derive_has_location!(GroupExpr);

impl Eval for GroupExpr {
    fn eval(&self) -> Result<Value, String> {
        let mut result = error(self, "Empty group");
        for e in &self.group {
            result = e.eval();
        }
        return result; // Return the result of the last evaluation
    }
}

impl ExprNode for GroupExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<bool, String> {
        self.group.push(Rc::clone(child));
        Ok(false)
    }
}

#[derive(Debug)]
struct Command {
    cmd: String,
    args: Vec<Rc<Expression>>,
    loc: Location,
}

derive_has_location!(Command);

impl Eval for Command {
    fn eval(&self) -> Result<Value, String> {
        if let Some(cmd) = get_command(&self.cmd) {
            let mut args: Vec<String> = Vec::new();
            for a in &self.args {
                let v = a.eval()?;
                args.push(format!("{}", v));
            }

            match cmd.exec(args) {
                Ok(v) => Ok(v),
                Err(e) => error(self, e.as_str()),
            }
        } else {
            panic!("Command not found");
        }
    }
}

impl ExprNode for Command {
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<bool, String> {
        assert!(!self.cmd.is_empty());
        self.args.push(Rc::clone(child));
        Ok(false)
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

fn eval_as_bool(expr: &Rc<Expression>) -> Result<bool, String> {
    match expr.eval()? {
        Value::Int(i) => Ok(i != 0),
        Value::Real(r) => Ok(r != 0.0),
        Value::Str(s) => Ok(!s.is_empty()),
    }
}

impl Eval for BranchExpr {
    fn eval(&self) -> Result<Value, String> {
        if self.cond.is_empty() {
            error(self, "Expecting IF condition")?;
        } else if self.if_branch.is_empty() {
            error(self, "Expecting IF branch")?;
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

impl ExprNode for BranchExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<bool, String> {
        if self.cond.is_empty() {
            self.cond = Rc::clone(child);
        } else if self.if_branch.is_empty() {
            self.if_branch = Rc::clone(child);
        } else if self.else_branch.is_empty() {
            if !self.expect_else {
                error(self, "Expecting ELSE keyword")?;
            }
            self.else_branch = Rc::clone(child);
            return Ok(true);
        } else {
            error(self, "Dangling expression after else branch")?;
        }
        Ok(false)
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
    fn eval(&self) -> Result<Value, String> {
        if self.cond.is_empty() {
            error(self, "Expecting loop conditional")?;
        } else if self.body.is_empty() {
            error(self, "Expecting loop body")?;
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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<bool, String> {
        if self.cond.is_empty() {
            self.cond = Rc::clone(child);
        } else if self.body.is_empty() {
            let expr = if child.is_group() {
                child
            } else {
                let g = new_group_cell(self.loc);
                g.borrow_mut().group.push(Rc::clone(&child));
                &Rc::new(Expression::Group(g))
            };
            self.body = Rc::clone(expr);
            return Ok(true); // Completed
        }

        Ok(false)
    }
}

#[derive(Clone, Debug)]
pub enum Value {
    Int(i64),
    Real(f64),
    Str(String),
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

trait Eval {
    fn eval(&self) -> Result<Value, String>;
}

impl Eval for Expression {
    fn eval(&self) -> Result<Value, String> {
        match &self {
            Expression::Bin(b) => b.borrow().eval(),
            Expression::Branch(b) => b.borrow().eval(),
            Expression::Cmd(c) => c.borrow().eval(),
            Expression::Group(g) => g.borrow().eval(),
            Expression::Empty => {
                panic!("Empty expression");
            }
            Expression::Lit(tok, scope) => match tok {
                Token::Literal(s) => {
                    if let Ok(i) = s.parse::<i64>() {
                        Ok(Value::Int(i))
                    } else if let Ok(f) = s.parse::<f64>() {
                        Ok(Value::Real(f))
                    } else if s.starts_with('$') {
                        match scope.lookup(&s[1..]) {
                            None => Err(format!("Variable not found: {}", s)),
                            Some(v) => Ok(v.value()),
                        }
                    } else {
                        Ok(Value::Str(s.to_owned()))
                    }
                }
                _ => {
                    panic!("Invalid token type in literal expression");
                }
            },
            Expression::Loop(l) => l.borrow().eval(),
        }
    }
}

pub struct Interp;

fn is_command(literal: &String) -> bool {
    get_command(&literal).is_some()
}

fn new_group_cell(loc: Location) -> RefCell<GroupExpr> {
    RefCell::new(GroupExpr {
        group: Vec::new(),
        loc: loc.clone(),
    })
}

fn new_group(loc: Location) -> Rc<Expression> {
    Rc::new(Expression::Group(new_group_cell(loc)))
}

impl Interp {
    pub fn eval(&mut self, input: &str) -> Result<Value, String> {
        let ast = self.parse(input)?;
        debug_dbg!(&ast);

        ast.eval()
    }

    fn parse(&mut self, input: &str) -> Result<Rc<Expression>, String> {
        let empty = Rc::new(Expression::Empty);
        let loc = Location { line: 1, col: 1 };
        let mut parser = Parser {
            chars: input.chars().peekable(),
            loc: loc,
            escaped: false,
            quoted: false,
            expect_else_expr: false,
            empty: Rc::clone(&empty),
            current_expr: Rc::clone(&empty),
            scope: Scope::new(None),
            expr_stack: Vec::new(),
            scope_stack: Vec::new(),
            group: new_group(loc),
            group_stack: Vec::new(),
        };

        loop {
            let tok = parser.next_token()?;
            match &tok {
                Token::End => {
                    break;
                }
                Token::LeftParen => {
                    parser.push(true);
                }
                Token::RightParen => {
                    if parser.expr_stack.is_empty() {
                        error(&parser, "Unmatched right parenthesis")?;
                    }
                    parser.pop()?;
                }
                Token::Semicolon => {
                    parser.add_current_expr_to_group();
                }
                Token::Literal(ref s) => {
                    // keywords
                    if s == "exit" || s == "quit" {
                        process::exit(0);
                    }
                    if s == "if" {
                        let expr = Rc::new(Expression::Branch(RefCell::new(BranchExpr {
                            cond: parser.empty(),
                            if_branch: parser.empty(),
                            else_branch: parser.empty(),
                            expect_else: false, // becomes true once "else" keyword is seen
                            loc: parser.loc,
                        })));
                        parser.add_expr(&expr)?;
                    } else if s == "else" {
                        if let Expression::Branch(b) = &*parser.current_expr {
                            if !b.borrow_mut().is_else_expected() {
                                error(&parser, "Conditional expression or IF branch missing")?;
                            }
                            parser.expect_else_expr = true;
                            parser.push(false);
                        } else {
                            error(&parser, "ELSE without IF")?;
                        }
                    } else if s == "while" {
                        let expr = Rc::new(Expression::Loop(RefCell::new(LoopExpr {
                            cond: parser.empty(),
                            body: parser.empty(),
                            loc: parser.loc,
                        })));
                        parser.add_expr(&expr)?;
                    // commands
                    } else if parser.current_expr.is_empty() && is_command(s) {
                        let expr = Rc::new(Expression::Cmd(RefCell::new(Command {
                            cmd: s.to_owned(),
                            args: Default::default(),
                            loc: parser.loc,
                        })));
                        parser.add_expr(&expr)?;
                    // identifiers and literals
                    } else {
                        let expr = Rc::new(Expression::Lit(tok, Rc::clone(&parser.scope)));
                        parser.add_expr(&expr)?;
                    }
                }
                Token::Operator(op) => {
                    if parser.current_expr.is_empty() {
                        error(&parser, "Expecting left-hand term in binary operation")?;
                    }
                    let expr = Rc::new(Expression::Bin(RefCell::new(BinExpr {
                        op: op.clone(),
                        lhs: Rc::clone(&parser.current_expr),
                        rhs: parser.empty(),
                        loc: parser.loc,
                        scope: Rc::clone(&parser.scope),
                    })));

                    parser.current_expr = expr;
                }
            }
        }
        if !parser.expr_stack.is_empty() {
            let msg = if parser.expect_else_expr {
                "Dangling else"
            } else {
                "Unbalanced parenthesis"
            };
            error(&parser, msg)?;
        }
        assert!(parser.group_stack.is_empty()); // because the expr_stack is empty

        parser.finalize_group();

        Ok(parser.group)
    }
}
