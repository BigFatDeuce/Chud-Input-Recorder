//! A tiny, safe expression language for Flow variables and conditions.
//!
//! ```text
//! count * 10 + 5        random(100, 250)        i < 10 && !done
//! "run " + i            max(a, b)               round(x / 3)
//! ```
//! No side effects, no loops, no I/O: evaluation always terminates.

use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Str(String),
    Bool(bool),
}

impl Value {
    pub fn truthy(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            Value::Bool(b) => *b,
        }
    }
    pub fn as_num(&self) -> Result<f64, ExprError> {
        match self {
            Value::Num(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Str(s) => s.trim().parse().map_err(|_| ExprError::Type(format!("\"{s}\" is not a number"))),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => write!(f, "{}", *n as i64),
            Value::Num(n) => write!(f, "{n}"),
            Value::Str(s) => write!(f, "{s}"),
            Value::Bool(b) => write!(f, "{b}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ExprError {
    #[error("syntax error at {pos}: {msg}")]
    Syntax { pos: usize, msg: String },
    #[error("unknown variable `{0}`")]
    UnknownVar(String),
    #[error("unknown function `{0}`")]
    UnknownFn(String),
    #[error("type error: {0}")]
    Type(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Lit(Value),
    Var(String),
    Unary(char, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

pub trait Env {
    fn var(&self, name: &str) -> Option<Value>;
    /// Uniform random number in `[0, 1)`.
    fn random(&mut self) -> f64;
}

impl Env for HashMap<String, Value> {
    fn var(&self, name: &str) -> Option<Value> {
        self.get(name).cloned()
    }
    fn random(&mut self) -> f64 {
        0.5
    }
}

// ---------------------------------------------------------------- lexer

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Str(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Comma,
}

fn lex(src: &str) -> Result<Vec<(usize, Tok)>, ExprError> {
    // Two-character operators first so that `<=` wins over `<`.
    const OPS: [(&str, &str); 17] = [
        ("&&", "&&"), ("||", "||"), ("==", "=="), ("!=", "!="), ("<=", "<="), (">=", ">="),
        ("+", "+"), ("-", "-"), ("*", "*"), ("/", "/"), ("%", "%"), ("<", "<"), (">", ">"),
        ("!", "!"), ("=", "=="), ("&", "&&"), ("|", "||"),
    ];
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let at = |k: usize| chars.get(k).map(|c| c.1);
    let byte = |k: usize| chars.get(k).map(|c| c.0).unwrap_or(src.len());
    let mut k = 0;
    let mut out = Vec::new();
    while k < chars.len() {
        let (pos, c) = chars[k];
        if c.is_whitespace() {
            k += 1;
        } else if c.is_ascii_digit() || (c == '.' && at(k + 1).is_some_and(|d| d.is_ascii_digit())) {
            let s = k;
            while at(k).is_some_and(|d| d.is_ascii_digit() || d == '.') {
                k += 1;
            }
            let text = &src[byte(s)..byte(k)];
            let n = text.parse().map_err(|_| ExprError::Syntax { pos, msg: format!("bad number `{text}`") })?;
            out.push((pos, Tok::Num(n)));
        } else if c == '"' || c == '\'' {
            k += 1;
            let mut s = String::new();
            loop {
                match at(k) {
                    None => return Err(ExprError::Syntax { pos, msg: "unterminated string".into() }),
                    Some(q) if q == c => break,
                    Some('\\') if at(k + 1).is_some() => {
                        s.push(at(k + 1).unwrap());
                        k += 2;
                    }
                    Some(ch) => {
                        s.push(ch);
                        k += 1;
                    }
                }
            }
            k += 1;
            out.push((pos, Tok::Str(s)));
        } else if c.is_alphabetic() || c == '_' {
            let s = k;
            while at(k).is_some_and(|d| d.is_alphanumeric() || d == '_') {
                k += 1;
            }
            out.push((pos, Tok::Ident(src[byte(s)..byte(k)].to_string())));
        } else if c == '(' {
            out.push((pos, Tok::LParen));
            k += 1;
        } else if c == ')' {
            out.push((pos, Tok::RParen));
            k += 1;
        } else if c == ',' {
            out.push((pos, Tok::Comma));
            k += 1;
        } else if let Some((raw, op)) = OPS.iter().find(|(raw, _)| src[pos..].starts_with(raw)) {
            out.push((pos, Tok::Op(op)));
            k += raw.len(); // operators are ASCII: one char per byte
        } else {
            return Err(ExprError::Syntax { pos, msg: format!("unexpected `{c}`") });
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- parser

struct Parser {
    toks: Vec<(usize, Tok)>,
    pos: usize,
    len: usize,
}

fn binding(op: &str) -> Option<(u8, u8)> {
    Some(match op {
        "||" => (1, 2),
        "&&" => (3, 4),
        "==" | "!=" => (5, 6),
        "<" | "<=" | ">" | ">=" => (7, 8),
        "+" | "-" => (9, 10),
        "*" | "/" | "%" => (11, 12),
        _ => return None,
    })
}

fn static_op(op: &str) -> &'static str {
    ["||", "&&", "==", "!=", "<", "<=", ">", ">=", "+", "-", "*", "/", "%"]
        .into_iter()
        .find(|o| *o == op)
        .unwrap_or("+")
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.1)
    }
    fn here(&self) -> usize {
        self.toks.get(self.pos).map(|t| t.0).unwrap_or(self.len)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).map(|t| t.1.clone());
        self.pos += 1;
        t
    }
    fn err<T>(&self, msg: &str) -> Result<T, ExprError> {
        Err(ExprError::Syntax { pos: self.here(), msg: msg.into() })
    }

    fn expr(&mut self, min_bp: u8) -> Result<Expr, ExprError> {
        let mut lhs = match self.next() {
            Some(Tok::Num(n)) => Expr::Lit(Value::Num(n)),
            Some(Tok::Str(s)) => Expr::Lit(Value::Str(s)),
            Some(Tok::Ident(id)) => match id.as_str() {
                "true" => Expr::Lit(Value::Bool(true)),
                "false" => Expr::Lit(Value::Bool(false)),
                _ if self.peek() == Some(&Tok::LParen) => {
                    self.next();
                    let mut args = Vec::new();
                    if self.peek() != Some(&Tok::RParen) {
                        loop {
                            args.push(self.expr(0)?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => return self.err("expected `,` or `)`"),
                            }
                        }
                    } else {
                        self.next();
                    }
                    Expr::Call(id, args)
                }
                _ => Expr::Var(id),
            },
            Some(Tok::Op("-")) => Expr::Unary('-', Box::new(self.expr(13)?)),
            Some(Tok::Op("!")) => Expr::Unary('!', Box::new(self.expr(13)?)),
            Some(Tok::LParen) => {
                let e = self.expr(0)?;
                if self.next() != Some(Tok::RParen) {
                    return self.err("expected `)`");
                }
                e
            }
            _ => {
                self.pos = self.pos.saturating_sub(1);
                return self.err("expected a value");
            }
        };
        while let Some(Tok::Op(op)) = self.peek() {
            let op = *op;
            let Some((l, r)) = binding(op) else { break };
            if l < min_bp {
                break;
            }
            self.next();
            let rhs = self.expr(r)?;
            lhs = Expr::Binary(static_op(op), Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
}

pub fn parse(src: &str) -> Result<Expr, ExprError> {
    let toks = lex(src)?;
    if toks.is_empty() {
        return Err(ExprError::Syntax { pos: 0, msg: "empty expression".into() });
    }
    let mut p = Parser { toks, pos: 0, len: src.len() };
    let e = p.expr(0)?;
    if p.pos < p.toks.len() {
        return p.err("unexpected token");
    }
    Ok(e)
}

// ---------------------------------------------------------------- eval

pub fn eval(e: &Expr, env: &mut dyn Env) -> Result<Value, ExprError> {
    Ok(match e {
        Expr::Lit(v) => v.clone(),
        Expr::Var(name) => env.var(name).ok_or_else(|| ExprError::UnknownVar(name.clone()))?,
        Expr::Unary('-', x) => Value::Num(-eval(x, env)?.as_num()?),
        Expr::Unary(_, x) => Value::Bool(!eval(x, env)?.truthy()),
        Expr::Binary(op, a, b) => {
            // Short-circuit logic first.
            if *op == "&&" {
                return Ok(Value::Bool(eval(a, env)?.truthy() && eval(b, env)?.truthy()));
            }
            if *op == "||" {
                return Ok(Value::Bool(eval(a, env)?.truthy() || eval(b, env)?.truthy()));
            }
            let (x, y) = (eval(a, env)?, eval(b, env)?);
            match *op {
                "+" => match (&x, &y) {
                    (Value::Str(_), _) | (_, Value::Str(_)) => Value::Str(format!("{x}{y}")),
                    _ => Value::Num(x.as_num()? + y.as_num()?),
                },
                "-" => Value::Num(x.as_num()? - y.as_num()?),
                "*" => Value::Num(x.as_num()? * y.as_num()?),
                "/" => Value::Num(x.as_num()? / y.as_num()?),
                "%" => Value::Num(x.as_num()? % y.as_num()?),
                "==" | "!=" => {
                    let eq = match (&x, &y) {
                        (Value::Str(p), Value::Str(q)) => p == q,
                        (Value::Bool(p), Value::Bool(q)) => p == q,
                        _ => match (x.as_num(), y.as_num()) {
                            (Ok(p), Ok(q)) => p == q,
                            _ => x.to_string() == y.to_string(),
                        },
                    };
                    Value::Bool(if *op == "==" { eq } else { !eq })
                }
                cmp => {
                    let (p, q) = (x.as_num()?, y.as_num()?);
                    Value::Bool(match cmp {
                        "<" => p < q,
                        "<=" => p <= q,
                        ">" => p > q,
                        _ => p >= q,
                    })
                }
            }
        }
        Expr::Call(name, args) => {
            let mut v = Vec::with_capacity(args.len());
            for a in args {
                v.push(eval(a, env)?);
            }
            let num = |i: usize| -> Result<f64, ExprError> {
                v.get(i).ok_or_else(|| ExprError::Type(format!("{name}: missing argument {}", i + 1)))?.as_num()
            };
            match name.as_str() {
                "random" => {
                    let (lo, hi) = (num(0)?, num(1)?);
                    Value::Num((lo + env.random() * (hi - lo)).round())
                }
                "rand" => Value::Num(env.random()),
                "min" => Value::Num(num(0)?.min(num(1)?)),
                "max" => Value::Num(num(0)?.max(num(1)?)),
                "abs" => Value::Num(num(0)?.abs()),
                "floor" => Value::Num(num(0)?.floor()),
                "ceil" => Value::Num(num(0)?.ceil()),
                "round" => Value::Num(num(0)?.round()),
                "clamp" => Value::Num(num(0)?.clamp(num(1)?, num(2)?.max(num(1)?))),
                _ => return Err(ExprError::UnknownFn(name.clone())),
            }
        }
    })
}

/// Parses and evaluates in one go.
pub fn eval_str(src: &str, env: &mut dyn Env) -> Result<Value, ExprError> {
    eval(&parse(src)?, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Value {
        let mut env: HashMap<String, Value> = HashMap::new();
        env.insert("i".into(), Value::Num(3.0));
        env.insert("name".into(), Value::Str("orbit".into()));
        eval_str(src, &mut env).unwrap()
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(run("1 + 2 * 3"), Value::Num(7.0));
        assert_eq!(run("(1 + 2) * 3"), Value::Num(9.0));
        assert_eq!(run("-i * 2"), Value::Num(-6.0));
        assert_eq!(run("10 % 4"), Value::Num(2.0));
        assert_eq!(run("random(100, 100)"), Value::Num(100.0));
        assert_eq!(run("max(i, 7) - min(1, 2)"), Value::Num(6.0));
    }

    #[test]
    fn logic_and_comparisons() {
        assert_eq!(run("i < 10 && i >= 3"), Value::Bool(true));
        assert_eq!(run("!(i == 3) || false"), Value::Bool(false));
        assert_eq!(run("i = 3"), Value::Bool(true));
        assert_eq!(run("name == 'orbit'"), Value::Bool(true));
    }

    #[test]
    fn strings_concatenate() {
        assert_eq!(run("\"run \" + i"), Value::Str("run 3".into()));
    }

    #[test]
    fn errors_are_reported() {
        let mut env: HashMap<String, Value> = HashMap::new();
        assert!(matches!(eval_str("1 +", &mut env), Err(ExprError::Syntax { .. })));
        assert!(matches!(eval_str("x + 1", &mut env), Err(ExprError::UnknownVar(_))));
        assert!(matches!(eval_str("nope(1)", &mut env), Err(ExprError::UnknownFn(_))));
        assert!(matches!(eval_str("(1", &mut env), Err(ExprError::Syntax { .. })));
        assert!(matches!(eval_str("", &mut env), Err(ExprError::Syntax { .. })));
    }
}
