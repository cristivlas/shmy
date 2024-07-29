use crate::cmds::{get_command, Exec};
use std::cell::RefCell;
use std::iter::Peekable;
use std::rc::Rc;
use std::{fmt, process};

fn is_reserved(c: char) -> bool {
    const RESERVED_CHARS: &str = " \t\n\r()+-";
    RESERVED_CHARS.contains(c)
}

#[derive(Clone, Debug, PartialEq)]
enum Op {
    Assign,
    Equals,
    Minus,
    Plus,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    End,
    Literal(String),
    Operator(Op),
    LeftParen,
    RightParen,
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

// Trait for objects with location info.
trait HasLocation {
    fn loc(&self) -> &Location;
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
    escaped: bool,
    quoted: bool,
    current: Rc<Expression>, // Current expression
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
                        return error(self, "Unrecognized token");
                    } else {
                        tok = Token::Literal(literal);
                    }
                }
            }
        }
        if self.quoted {
            return error(self, "Unbalanced quotes");
        }
        Ok(tok)
    }

    /// Add an expression to the AST.
    fn add_expr(&mut self, expr: &Rc<Expression>) -> Result<(), String> {
        if expr.is_empty() {
            return error(self, "Unexpected empty expression");
        }
        let ref current = *self.current;
        match current {
            Expression::Bin(e) => {
                return e.borrow_mut().add_child(expr);
            }
            Expression::Cmd(e) => {
                return e.borrow_mut().add_child(expr);
            }
            Expression::Empty => {
                self.current = Rc::clone(expr);
            }
            Expression::Lit(_) => {
                return error(self, "Expression after literal");
            }
        }

        Ok(())
    }

    fn is_arg_expected(&self) -> bool {
        let ref current = *self.current;
        match current {
            Expression::Cmd(_) => {
                return true;
            }
            _ => {}
        }
        false
    }
}

#[derive(Clone, Debug)]
enum Expression {
    Empty,
    Bin(RefCell<BinExpr>),
    Cmd(RefCell<Command>),
    Lit(Token),
}

impl Expression {
    fn is_empty(&self) -> bool {
        matches!(self, Expression::Empty)
    }
}

#[derive(Clone, Debug)]
struct BinExpr {
    op: Op,
    lhs: Rc<Expression>,
    rhs: Rc<Expression>,
    loc: Location,
}

impl HasLocation for BinExpr {
    fn loc(&self) -> &Location {
        &self.loc
    }
}

impl ExprNode for BinExpr {
    /// Add right hand-side child expression.
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
        assert!(!self.lhs.is_empty());

        if self.rhs.is_empty() {
            self.rhs = Rc::clone(child);
            Ok(())
        } else {
            if let Expression::Lit(Token::Literal(s)) = &*self.rhs {
                if is_command(s) {
                    let cmd = Expression::Cmd(RefCell::new(Command {
                        cmd: s.to_owned(),
                        args: vec![Rc::clone(child)],
                        loc: self.loc,
                    }));
                    self.rhs = Rc::new(cmd);
                    return Ok(());
                }
            }
            error(self, "Unexpected dangling expression")
        }
    }
}

impl BinExpr {
    fn eval_plus(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => {
                    return Ok(Value::Int(i + j));
                }
                Value::Real(j) => {
                    return Ok(Value::Real(i as f64 + j));
                }
                Value::Str(ref s) => {
                    return Ok(Value::Str(format!("{}{}", i, s)));
                }
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => {
                    return Ok(Value::Real(i + j as f64));
                }
                Value::Real(j) => {
                    return Ok(Value::Real(i + j));
                }
                Value::Str(ref s) => {
                    return Ok(Value::Str(format!("{}{}", i, s)));
                }
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

    fn eval_minus(&self, lhs: Value, rhs: Value) -> Result<Value, String> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => {
                    return Ok(Value::Int(i - j));
                }
                Value::Real(j) => {
                    return Ok(Value::Real(i as f64 - j));
                }
                Value::Str(_) => {
                    return error(self, "Can't subtract string from integer");
                }
            },
            Value::Real(i) => match rhs {
                Value::Int(j) => {
                    return Ok(Value::Real(i - j as f64));
                }
                Value::Real(j) => {
                    return Ok(Value::Real(i - j));
                }
                Value::Str(_) => {
                    return error(self, "Can't subtract string from number");
                }
            },
            Value::Str(_) => match rhs {
                Value::Int(_) | Value::Real(_) => {
                    return error(self, "Can't subtract number from string");
                }
                Value::Str(_) => {
                    return error(self, "Can't subtract strings");
                }
            },
        }
    }
}

impl Eval for BinExpr {
    fn eval(&self) -> Result<Value, String> {
        assert!(!self.lhs.is_empty());
        assert!(!self.rhs.is_empty());

        let lhs = self.lhs.eval()?;
        let rhs = self.rhs.eval()?;

        match self.op {
            Op::Assign => Ok(Value::Str("TODO: Assign".to_owned())),
            Op::Equals => Ok(Value::Str("TODO: Equals".to_owned())),
            Op::Minus => self.eval_minus(lhs, rhs),
            Op::Plus => self.eval_plus(lhs, rhs),
        }
    }
}

#[derive(Clone, Debug)]
struct Command {
    cmd: String,
    args: Vec<Rc<Expression>>,
    loc: Location,
}

impl HasLocation for Command {
    fn loc(&self) -> &Location {
        &self.loc
    }
}

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
    fn add_child(&mut self, child: &Rc<Expression>) -> Result<(), String> {
        assert!(!self.cmd.is_empty());
        self.args.push(Rc::clone(child));
        Ok(())
    }
}

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
            Expression::Cmd(c) => c.borrow().eval(),
            Expression::Empty => {
                panic!("Empty expression");
            }
            Expression::Lit(t) => match &t {
                Token::Literal(s) => {
                    if let Ok(i) = s.parse::<i64>() {
                        Ok(Value::Int(i))
                    } else if let Ok(f) = s.parse::<f64>() {
                        Ok(Value::Real(f))
                    } else {
                        Ok(Value::Str(s.to_owned()))
                    }
                }
                _ => {
                    panic!("Invalid token in literal eval");
                }
            },
        }
    }
}

pub struct Interp;

fn is_command(literal: &String) -> bool {
    get_command(&literal).is_some()
}

impl Interp {
    pub fn eval(&mut self, input: &str) -> Result<Value, String> {
        let mut parser = Parser {
            chars: input.chars().peekable(),
            loc: Location { line: 1, col: 1 },
            escaped: false,
            quoted: false,
            current: Rc::new(Expression::Empty),
        };

        let mut stack: Vec<Rc<Expression>> = Vec::new();

        loop {
            let tok = parser.next_token()?;
            match &tok {
                Token::End => {
                    break;
                }
                Token::LeftParen => {
                    stack.push(Rc::clone(&parser.current));
                    parser.current = Rc::new(Expression::Empty);
                }
                Token::RightParen => {
                    if stack.is_empty() {
                        return error(&parser, "Unmatched closed parenthesis");
                    }
                    let expr = parser.current;
                    parser.current = stack.pop().unwrap();
                    parser.add_expr(&expr)?;
                }
                Token::Literal(ref s) => {
                    if s == "exit" || s == "quit" {
                        process::exit(0);
                    }
                    if parser.current.is_empty() && is_command(s) {
                        let expr = Rc::new(Expression::Cmd(RefCell::new(Command {
                            cmd: s.to_owned(),
                            args: Default::default(),
                            loc: parser.loc,
                        })));
                        parser.add_expr(&expr)?;
                    } else {
                        let expr = Rc::new(Expression::Lit(tok));
                        parser.add_expr(&expr)?;
                    }
                }
                Token::Operator(op) => {
                    if parser.current.is_empty() {
                        return error(&parser, "Missing left-hand term in operation");
                    }
                    parser.current = Rc::new(Expression::Bin(RefCell::new(BinExpr {
                        op: op.clone(),
                        lhs: parser.current.clone(),
                        rhs: Rc::new(Expression::Empty),
                        loc: parser.loc,
                    })));
                }
            }
        }
        if !stack.is_empty() {
            return error(&parser, "Unmatched parenthesis");
        }

        let ref ast_root = *parser.current;
        dbg!(ast_root);
        ast_root.eval()
    }
}
