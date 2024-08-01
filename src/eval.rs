use crate::cmds::{get_command, Exec};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::env;
use std::iter::Peekable;
use std::rc::Rc;
use std::str::FromStr;
use std::{fmt, process};

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

impl HasLocation for Location {
    fn loc(&self) -> &Location {
        self
    }
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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String>;
}

struct Parser<I: Iterator<Item = char>> {
    chars: Peekable<I>,
    loc: Location,
    comment: bool,
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

/// Tokenizer helper.
///
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

/// Parser implementation
///
impl<T> Parser<T>
where
    T: Iterator<Item = char>,
{
    fn empty(&self) -> Rc<Expression> {
        Rc::clone(&self.empty)
    }

    fn is_reserved(&self, c: char) -> bool {
        if c == '/' {
            // Treat forward slashes as regular chars in argument to commands.
            !self.current_expr.is_cmd()
        } else {
            const RESERVED_CHARS: &str = " \t\n\r()+-=;*|&<>!";
            RESERVED_CHARS.contains(c)
        }
    }

    fn next(&mut self) {
        self.loc.col += 1;
        self.chars.next();
    }

    #[rustfmt::skip]
    pub fn next_token(&mut self) -> Result<Token, String> {
        let mut tok = Token::End;
        let mut literal = String::new();

        while let Some(c) = self.chars.peek() {
            if tok != Token::End {
                break;
            }
            if *c == '\n' {
                self.loc.line += 1;
                self.loc.col = 0;
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
                '*' => token!(self, tok, Token::Operator(Op::Mul)),
                '&' => token!(self, tok, '&', Token::Operator(Op::And)),
                '|' => token!(self, tok, '|', Token::Operator(Op::Pipe), Token::Operator(Op::Or)),
                '!' => token!(self, tok, '=', Token::Operator(Op::NotEquals)),
                '<' => token!(self, tok, '=', Token::Operator(Op::Lt), Token::Operator(Op::Lte)),
                '>' => token!(self, tok, '=', Token::Operator(Op::Gt), Token::Operator(Op::Gte)),
                '=' => token!(self, tok, '=', Token::Operator(Op::Assign), Token::Operator(Op::Equals)),
                '-' => { if self.current_expr.is_cmd() {
                        literal.push(*c);
                    } else {
                        tok = Token::Operator(Op::Minus);
                    }
                    self.next();
                }
                '/' => if self.current_expr.is_cmd() {
                    // Treat forward slashes as chars in arguments to commands, to avoid quoting file paths.
                        literal.push(*c);
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
                    let mut has_chars = false;

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
                        if self.quoted || self.escaped || !self.is_reserved(next_c) {
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
                        tok = Token::Literal(literal.clone());
                        literal.clear();
                    }
                }
            }
        }
        if self.quoted {
            error(self, "Unbalanced quotes")?;
        }

        // Check for partial token, to handle special cases such as single fwd slash
        if tok == Token::End && !literal.is_empty() {
            tok = Token::Literal(literal);
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
            Expression::Bin(e) => e.borrow_mut().add_child(expr),
            Expression::Branch(e) => e.borrow_mut().add_child(expr),
            Expression::Cmd(e) => e.borrow_mut().add_child(expr),
            Expression::Empty => {
                self.current_expr = Rc::clone(expr);
                Ok(())
            }
            Expression::Group(e) => e.borrow_mut().add_child(expr),
            Expression::Lit(_) => error(self, "Dangling expression after literal"),
            Expression::Loop(e) => e.borrow_mut().add_child(expr),
        }
    }

    fn add_current_expr_to_group(&mut self) -> Result<(), String> {
        if !self.current_expr.is_empty() {
            if let Expression::Group(g) = &*Rc::clone(&self.group) {
                while let Some(stack_top) = self.expr_stack.last() {
                    if stack_top.is_bin_op() {
                        let expr = Rc::clone(&self.current_expr);
                        self.current_expr = self.expr_stack.pop().unwrap();
                        self.add_expr(&expr)?;
                    } else {
                        break;
                    }
                }
                g.borrow_mut().group.push(Rc::clone(&self.current_expr));
            } else {
                panic!("Unexpected group error");
            }
            self.current_expr = self.empty(); // Clear the current expression
        }
        Ok(())
    }

    fn finalize_group(&mut self) -> Result<(), String> {
        self.add_current_expr_to_group()
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
        self.finalize_group()?;
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

impl From<&str> for Variable {
    fn from(value: &str) -> Self {
        Variable {
            val: Rc::new(RefCell::new(value.parse::<Value>().unwrap())),
        }
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

    fn new_from_env() -> Rc<Scope> {
        let vars = env::vars()
            .map(|(key, value)| (key, Variable::from(value.as_str())))
            .collect::<HashMap<_, _>>();

        Rc::new(Scope {
            parent: None,
            vars: RefCell::new(vars),
        })
    }
}

fn parse_value(s: &str, scope: &Rc<Scope>, loc: &Location) -> Result<Value, String> {
    if s.starts_with('$') {
        match scope.lookup(&s[1..]) {
            None => error(loc, &format!("Variable not found: {}", s)),
            Some(v) => Ok(v.value()),
        }
    } else {
        s.parse::<Value>()
    }
}

#[derive(Debug)]
enum Expression {
    Empty,
    Bin(RefCell<BinExpr>),
    Cmd(RefCell<Command>),
    Branch(RefCell<BranchExpr>),
    Group(RefCell<GroupExpr>),
    Lit(Rc<Literal>),
    Loop(RefCell<LoopExpr>),
}

impl Expression {
    fn is_bin_op(&self) -> bool {
        matches!(self, Expression::Bin(_))
    }

    fn is_cmd(&self) -> bool {
        matches!(self, Expression::Cmd(_))
    }

    fn is_empty(&self) -> bool {
        matches!(self, Expression::Empty)
    }

    fn is_group(&self) -> bool {
        matches!(self, Expression::Group(_))
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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
        if self.rhs.is_empty() {
            self.rhs = Rc::clone(child);
            Ok(())
        } else {
            error(self, "Dangling expression")
        }
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
            Value::Str(_) => error($self, "Cannot divide number by string"),
        }
    };
}

/// Macro to generate comparison functions
macro_rules! eval_cmp_fn {
    ($fn_name:ident, $op:tt) => {
        fn $fn_name(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
            match self.eval_cmp(lhs, rhs)? {
                Value::Real(r) => Ok(Value::Int((r $op 0.0) as i64)),
                _ => panic!("Unexpected result type in comparison"),
            }
        }
    }
}

impl BinExpr {
    fn eval_and(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
        Ok(Value::Int((value_as_bool(lhs) && value_as_bool(rhs)) as _))
    }

    fn eval_assign(&self, rhs: Value) -> Result<Value, String> {
        if let Expression::Lit(lit) = &*self.lhs {
            if let Token::Literal(name) = &lit.tok {
                if name.starts_with('$') {
                    if let Some(var) = lit.scope.lookup(&name[1..]) {
                        var.assign(rhs);
                        return Ok(var.value());
                    } else {
                        error(self, &format!("Variable not found: {}", name))?;
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

    fn eval_div(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
        match lhs {
            Value::Int(i) => div_match!(self, i, rhs),
            Value::Real(i) => div_match!(self, i, rhs),
            Value::Str(s1) => match rhs {
                Value::Int(_) | Value::Real(_) => error(self, "Cannot divide string by number"),
                Value::Str(s2) => Ok(Value::Str(format!("{}/{}", s1, s2))),
            },
        }
    }

    fn eval_int_div(&self, _lhs: Value, _rhs: Value) -> Result<Value, String> {
        Err("NOT IMPLEMENTED".to_string())
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

    fn eval_mod(&self, _lhs: Value, _rhs: Value) -> Result<Value, String> {
        Err("NOT IMPLEMENTED".to_string())
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

    fn eval_or(&self, _lhs: Value, _rhs: Value) -> Result<Value, String> {
        Err("NOT IMPLEMENTED".to_string())
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
        if self.rhs.is_empty() {
            error(self, "Expecting right hand-side expression")?;
        }

        if self.op == Op::Pipe {
            if self.lhs.is_empty() {
                return error(self, "Expecting pipe input");
            }
            error(self, "Pipes are not implemented")?;
        }

        let rhs = self.rhs.eval()?;

        if self.lhs.is_empty() {
            eval_unary(&self.loc, &self.op, rhs)
        } else {
            match self.op {
                Op::And => self.eval_and(self.lhs.eval()?, rhs),
                Op::Assign => self.eval_assign(rhs.clone()),
                Op::Div => self.eval_div(self.lhs.eval()?, rhs),
                Op::Gt => self.eval_gt(self.lhs.eval()?, rhs),
                Op::Gte => self.eval_gte(self.lhs.eval()?, rhs),
                Op::IntDiv => self.eval_int_div(self.lhs.eval()?, rhs),
                Op::Equals => self.eval_equals(self.lhs.eval()?, rhs),
                Op::Lt => self.eval_lt(self.lhs.eval()?, rhs),
                Op::Lte => self.eval_lte(self.lhs.eval()?, rhs),
                Op::Minus => self.eval_minus(self.lhs.eval()?, rhs),
                Op::Mod => self.eval_mod(self.lhs.eval()?, rhs),
                Op::Mul => self.eval_mul(self.lhs.eval()?, rhs),
                Op::NotEquals => self.eval_not_equals(self.lhs.eval()?, rhs),
                Op::Or => self.eval_or(self.lhs.eval()?, rhs),
                Op::Plus => self.eval_plus(self.lhs.eval()?, rhs),
                _ => {
                    dbg!(&self.op);
                    return error(self, "Unexpected operator");
                }
            }
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
            if result.is_err() {
                break;
            }
        }
        result // return the last evaluation
    }
}

impl ExprNode for GroupExpr {
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
        self.group.push(Rc::clone(child));
        Ok(())
    }
}

#[derive(Debug)]
struct Command {
    cmd: String,
    args: Vec<Rc<Expression>>,
    loc: Location,
}

derive_has_location!(Command);

impl Command {
    fn exec(&self) -> Result<Value, String> {
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

impl Eval for Command {
    fn eval(&self) -> Result<Value, String> {
        self.exec()
    }
}

impl ExprNode for Command {
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
        assert!(!self.cmd.is_empty());
        self.args.push(Rc::clone(child));
        Ok(())
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

fn eval_as_bool(expr: &Rc<Expression>) -> Result<bool, String> {
    Ok(value_as_bool(expr.eval()?))
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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
        if self.cond.is_empty() {
            self.cond = Rc::clone(child);
        } else if self.if_branch.is_empty() {
            self.if_branch = Rc::clone(child);
        } else if self.else_branch.is_empty() {
            if !self.expect_else {
                error(self, "Expecting ELSE keyword")?;
            }
            self.else_branch = Rc::clone(child);
        } else {
            error(self, "Dangling expression after else branch")?;
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
    fn eval(&self) -> Result<Value, String> {
        match &self.tok {
            Token::Literal(s) => parse_value(&s, &self.scope, &self.loc),
            _ => {
                panic!("Invalid token type in literal expression");
            }
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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
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
        }
        Ok(())
    }
}

fn eval_unary(loc: &Location, op: &Op, val: Value) -> Result<Value, String> {
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
    type Err = String;

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
            Expression::Lit(lit) => lit.eval(),
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
        debug_print!(&ast);

        ast.eval()
    }

    fn parse(&mut self, input: &str) -> Result<Rc<Expression>, String> {
        let empty = Rc::new(Expression::Empty);
        let loc = Location { line: 1, col: 0 };
        let mut parser = Parser {
            chars: input.chars().peekable(),
            loc: loc,
            comment: false,
            escaped: false,
            quoted: false,
            expect_else_expr: false,
            empty: Rc::clone(&empty),
            current_expr: Rc::clone(&empty),
            scope: Scope::new_from_env(),
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
                    if parser.group_stack.is_empty() {
                        error(&parser, "Unmatched right parenthesis")?;
                    }
                    parser.pop()?;
                }
                Token::Semicolon => {
                    parser.add_current_expr_to_group()?;
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
                        let expr = Rc::new(Expression::Lit(Rc::new(Literal {
                            tok,
                            loc: parser.loc,
                            scope: Rc::clone(&parser.scope),
                        })));
                        parser.add_expr(&expr)?;
                    }
                }
                Token::Operator(op) => {
                    let expr = Rc::new(Expression::Bin(RefCell::new(BinExpr {
                        op: op.clone(),
                        lhs: Rc::clone(&parser.current_expr),
                        rhs: parser.empty(),
                        loc: parser.loc,
                        scope: Rc::clone(&parser.scope),
                    })));

                    match op {
                        // Right hand-side associative operations.
                        Op::Assign
                        | Op::Gt
                        | Op::Gte
                        | Op::Lt
                        | Op::Lte
                        | Op::NotEquals
                        | Op::Minus
                        | Op::Pipe
                        | Op::Plus => {
                            parser.expr_stack.push(Rc::clone(&expr));
                            parser.current_expr = parser.empty();
                        }
                        _ => {
                            parser.current_expr = expr;
                        }
                    }
                }
            }
        }

        parser.finalize_group()?;

        if !parser.expr_stack.is_empty() {
            let msg = if parser.expect_else_expr {
                "Dangling else"
            } else {
                dbg!(&parser.expr_stack);
                "Unbalanced parenthesis"
            };
            error(&parser, msg)?;
        }
        assert!(parser.group_stack.is_empty()); // because the expr_stack is empty

        Ok(parser.group)
    }
}
