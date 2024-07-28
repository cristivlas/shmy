use std::iter::Peekable;
use std::rc::Rc;
use std::{fmt, process};

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

struct Location {
    line: u32,
    col: u32,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[{}:{}]", self.line, self.col)
    }
}

struct Parser<I: Iterator<Item = char>> {
    chars: Peekable<I>,
    loc: Location,
    escaped: bool,
    quoted: bool,
    current: Rc<Expression>, // Current expression
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
                        if self.quoted
                            || self.escaped
                            || !matches!(next_c, ' ' | '\t' | '\n' | '\r' | '(' | ')')
                        {
                            literal.push(next_c);
                            self.next();
                        } else {
                            break;
                        }
                        self.escaped = false;
                    }
                    if has_chars && literal.is_empty() {
                        return self.error("Unrecognized token");
                    } else {
                        tok = Token::Literal(literal);
                    }
                }
            }
        }
        if self.quoted {
            return self.error("Unbalanced quotes");
        }
        Ok(tok)
    }

    // Add an expression to the AST.
    fn add_expr(&mut self, expr: &Rc<Expression>) -> Result<(), String> {
        if expr.is_empty() {
            return self.error("Unexpected empty expression");
        }
        match &*self.current {
            Expression::Bin(e) => {
                self.current = e.add_child(expr);
            }
            Expression::Cmd(e) => {
                self.current = e.add_child(expr);
            }
            Expression::Empty => {
                self.current = Rc::clone(expr);
            }
            Expression::Lit(_) => {
                return self.error("Expression after literal");
            }
        }

        Ok(())
    }

    fn error<R>(&self, s: &str) -> Result<R, String> {
        return Err(format!("{} {}", self.loc, s));
    }

    fn is_arg_expected(&self) -> bool {
        match &*self.current {
            Expression::Cmd(_) => {
                return true;
            }
            _ => {}
        }
        false
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Expression {
    Empty,
    Bin(BinExpr),
    Cmd(Command),
    Lit(Token),
}

impl Expression {
    fn is_empty(&self) -> bool {
        *self == Expression::Empty
    }
}

trait ExprNode {
    fn add_child(&self, child: &Rc<Expression>) -> Rc<Expression>;
}

#[derive(Clone, Debug, PartialEq)]
struct BinExpr {
    op: Op,
    lhs: Rc<Expression>,
    rhs: Rc<Expression>,
}

impl ExprNode for BinExpr {
    fn add_child(&self, child: &Rc<Expression>) -> Rc<Expression> {
        assert!(!self.lhs.is_empty());
        assert!(self.rhs.is_empty());
        let mut e = self.clone();
        e.rhs = Rc::clone(child);
        Rc::new(Expression::Bin(e))
    }
}

impl BinExpr {
    fn eval_plus(lhs: Value, rhs: Value) -> Result<Value, String> {
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

    fn eval_minus(lhs: Value, rhs: Value) -> Result<Value, String> {
        match lhs {
            Value::Int(i) => match rhs {
                Value::Int(j) => {
                    return Ok(Value::Int(i - j));
                }
                Value::Real(j) => {
                    return Ok(Value::Real(i as f64 - j));
                }
                Value::Str(_) => {
                    return Err("Can't subtract string from integer".to_owned());
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
                    return Err("Can't subtract string from number".to_owned());
                }
            },
            Value::Str(_) => match rhs {
                Value::Int(_) | Value::Real(_) => {
                    return Err("Can't subtract number from string".to_owned());
                }
                Value::Str(_) => {
                    return Err("Can't subtract strings".to_owned());
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
            Op::Assign => {
                return Ok(Value::Str("TODO: Assign".to_owned()));
            }
            Op::Equals => {
                return Ok(Value::Str("TODO: Equals".to_owned()));
            }
            Op::Minus => {
                return BinExpr::eval_minus(lhs, rhs);
            }
            Op::Plus => BinExpr::eval_plus(lhs, rhs),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Command {
    cmd: String,
    args: Vec<Rc<Expression>>,
}

impl Eval for Command {
    fn eval(&self) -> Result<Value, String> {
        Ok(Value::Str("TODO: Command".to_string()))
    }
}

impl ExprNode for Command {
    fn add_child(&self, child: &Rc<Expression>) -> Rc<Expression> {
        assert!(!self.cmd.is_empty());
        let mut e = self.clone();
        e.args.push(Rc::clone(child));
        Rc::new(Expression::Cmd(e))
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
            Expression::Bin(b) => b.eval(),
            Expression::Cmd(c) => c.eval(),
            Expression::Empty => {
                return Err("Empty expression".to_owned());
            }
            Expression::Lit(t) => match &t {
                Token::Literal(s) => {
                    let i = s.parse::<i64>();
                    if i.is_ok() {
                        return Ok(Value::Int(i.unwrap()));
                    }
                    let f = s.parse::<f64>();
                    if f.is_ok() {
                        return Ok(Value::Real(f.unwrap()));
                    }
                    return Ok(Value::Str(s.to_owned()));
                }
                _ => {
                    return Err("Invalid token in literal eval".to_owned());
                }
            },
        }
    }
}

pub struct Interp;

impl Interp {
    fn is_command(&self, literal: &String) -> bool {
        if literal == "ls" {
            return true;
        }
        false
    }

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
                        return parser.error("Unmatched closed parenthesis");
                    }
                    let expr = parser.current;
                    parser.current = stack.pop().unwrap();
                    parser.add_expr(&expr)?;
                }
                Token::Literal(ref s) => {
                    if s == "exit" {
                        process::exit(0);
                    }
                    if parser.current.is_empty() && self.is_command(s) {
                        let expr = Rc::new(Expression::Cmd(Command {
                            cmd: s.to_owned(),
                            args: Default::default(),
                        }));
                        parser.add_expr(&expr)?;
                    } else {
                        let expr = Rc::new(Expression::Lit(tok));
                        parser.add_expr(&expr)?;
                    }
                }
                Token::Operator(op) => {
                    if parser.current.is_empty() {
                        return parser.error("Missing left-hand term in operation");
                    }
                    parser.current = Rc::new(Expression::Bin(BinExpr {
                        op: op.clone(),
                        lhs: parser.current.clone(),
                        rhs: Rc::new(Expression::Empty),
                    }));
                }
            }
        }
        if !stack.is_empty() {
            return parser.error("Unmatched parenthesis");
        }
        dbg!(&parser.current);
        parser.current.eval()
    }
}
