//! The integer arithmetic of `#if` and `#elif` (spec 1.5, GLSL 4.60 §3.3): `defined`, integer
//! literals, macros expanded to their values, and the C operators with C precedence.
//! An identifier that is not a macro counts as 0, as in C: GLSL would reject the line, and a
//! pack would then lose a whole section of its settings over one unknown name.

use crate::Macros;
use crate::macros::{is_ident, is_ident_start};

/// The value of a condition; `Err` says what is wrong with it.
pub fn eval_condition(expr: &str, macros: &Macros) -> Result<i64, String> {
    let tokens = tokens(&resolve_defined(expr, macros)?, macros)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
    };
    let value = parser.binary(0)?;
    match parser.tokens.get(parser.at) {
        None => Ok(value),
        Some(t) => Err(format!("unexpected `{}`", t.text())),
    }
}

/// `defined X` and `defined(X)` become `1` or `0` before anything is expanded.
fn resolve_defined(expr: &str, macros: &Macros) -> Result<String, String> {
    let mut out = String::with_capacity(expr.len());
    let mut rest = expr;
    while let Some(at) = find_word(rest, "defined") {
        out.push_str(&rest[..at]);
        let after = rest[at + "defined".len()..].trim_start();
        let (inner, tail) = match after.strip_prefix('(') {
            Some(open) => {
                let close = open.find(')').ok_or("`defined(` without `)`")?;
                (open[..close].trim(), &open[close + 1..])
            }
            None => {
                let len = after.find(|c: char| !is_ident(c)).unwrap_or(after.len());
                (&after[..len], &after[len..])
            }
        };
        if inner.is_empty() || !inner.starts_with(is_ident_start) || !inner.chars().all(is_ident) {
            return Err("`defined` needs a macro name".into());
        }
        out.push_str(if macros.is_defined(inner) {
            " 1 "
        } else {
            " 0 "
        });
        rest = tail;
    }
    out.push_str(rest);
    Ok(out)
}

/// Where `word` starts in `text` as a whole identifier.
fn find_word(text: &str, word: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(i) = text[from..].find(word).map(|i| i + from) {
        let before = text[..i].chars().next_back().is_some_and(is_ident);
        let after = text[i + word.len()..].chars().next().is_some_and(is_ident);
        if !before && !after {
            return Some(i);
        }
        from = i + word.len();
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Num(i64),
    Op(&'static str),
}

impl Token {
    fn text(&self) -> String {
        match self {
            Token::Num(n) => n.to_string(),
            Token::Op(op) => (*op).to_owned(),
        }
    }
}

const OPS: [&str; 22] = [
    "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "(", ")", "!", "~", "+", "-", "*", "/", "%",
    "<", ">", "&", "^", "|",
];

fn tokens(expr: &str, macros: &Macros) -> Result<Vec<Token>, String> {
    let expanded = macros.expand(expr);
    let mut out = Vec::new();
    let mut rest = expanded.trim_start();
    while let Some(c) = rest.chars().next() {
        if c.is_ascii_digit() {
            let len = rest
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(rest.len());
            if rest[len..].starts_with('.') {
                return Err(format!("`{}`: `#if` takes integers only", &rest[..len + 1]));
            }
            out.push(Token::Num(int_literal(&rest[..len])?));
            rest = &rest[len..];
        } else if is_ident_start(c) {
            // Not a macro: 0, as the C preprocessor counts it.
            let len = rest.find(|c: char| !is_ident(c)).unwrap_or(rest.len());
            out.push(Token::Num(0));
            rest = &rest[len..];
        } else {
            let op = OPS
                .iter()
                .find(|op| rest.starts_with(**op))
                .ok_or_else(|| format!("unexpected `{c}`"))?;
            out.push(Token::Op(op));
            rest = &rest[op.len()..];
        }
        rest = rest.trim_start();
    }
    Ok(out)
}

/// Decimal, `0x` hexadecimal or leading-zero octal, with an optional `u` suffix.
fn int_literal(text: &str) -> Result<i64, String> {
    let digits = text.strip_suffix(['u', 'U']).unwrap_or(text);
    let parsed = if let Some(hex) = digits.strip_prefix("0x").or(digits.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16)
    } else if digits.len() > 1 && digits.starts_with('0') {
        i64::from_str_radix(&digits[1..], 8)
    } else {
        digits.parse()
    };
    parsed.map_err(|_| format!("`{text}` is not an integer"))
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

/// Binary operators from the loosest to the tightest (GLSL §5.1 / §3.3).
const LEVELS: [&[&str]; 10] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];

impl Parser<'_> {
    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).copied();
        self.at += 1;
        t
    }

    fn binary(&mut self, level: usize) -> Result<i64, String> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(&Token::Op(op)) = self.tokens.get(self.at) {
            if !LEVELS[level].contains(&op) {
                break;
            }
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = apply(op, left, right)?;
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<i64, String> {
        match self.next() {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::Op("(")) => {
                let value = self.binary(0)?;
                match self.next() {
                    Some(Token::Op(")")) => Ok(value),
                    _ => Err("missing `)`".into()),
                }
            }
            Some(Token::Op("-")) => Ok(self.unary()?.wrapping_neg()),
            Some(Token::Op("+")) => self.unary(),
            Some(Token::Op("!")) => Ok((self.unary()? == 0) as i64),
            Some(Token::Op("~")) => Ok(!self.unary()?),
            Some(t) => Err(format!("unexpected `{}`", t.text())),
            None => Err("the condition ends too early".into()),
        }
    }
}

fn apply(op: &str, a: i64, b: i64) -> Result<i64, String> {
    Ok(match op {
        "||" => (a != 0 || b != 0) as i64,
        "&&" => (a != 0 && b != 0) as i64,
        "|" => a | b,
        "^" => a ^ b,
        "&" => a & b,
        "==" => (a == b) as i64,
        "!=" => (a != b) as i64,
        "<" => (a < b) as i64,
        ">" => (a > b) as i64,
        "<=" => (a <= b) as i64,
        ">=" => (a >= b) as i64,
        "<<" => a.wrapping_shl(b as u32),
        ">>" => a.wrapping_shr(b as u32),
        "+" => a.wrapping_add(b),
        "-" => a.wrapping_sub(b),
        "*" => a.wrapping_mul(b),
        "/" | "%" if b == 0 => return Err("division by zero".into()),
        "/" => a.wrapping_div(b),
        _ => a.wrapping_rem(b),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> Result<i64, String> {
        let mut m = Macros::standard();
        m.define("SHADOW_RES", "2048");
        m.define("WAVES", "");
        m.define("QUALITY", "SHADOW_RES / 1024");
        eval_condition(expr, &m)
    }

    #[test]
    fn precedence_and_operators_follow_c() {
        assert_eq!(eval("1 + 2 * 3"), Ok(7));
        assert_eq!(eval("(1 + 2) * 3"), Ok(9));
        assert_eq!(eval("1 << 2 + 1"), Ok(8));
        assert_eq!(eval("6 & 3 == 3"), Ok(0));
        assert_eq!(eval("1 || 0 && 0"), Ok(1));
        assert_eq!(eval("-7 / 2"), Ok(-3));
        assert_eq!(eval("-7 % 2"), Ok(-1));
        assert_eq!(eval("!0 + ~0"), Ok(0));
        assert_eq!(eval("0x10 + 010 + 3u"), Ok(27));
    }

    #[test]
    fn macros_and_defined() {
        assert_eq!(eval("MC_VERSION >= 11300"), Ok(1));
        assert_eq!(eval("MC_VERSION <= 12111"), Ok(0));
        assert_eq!(eval("QUALITY == 2"), Ok(1));
        assert_eq!(eval("defined WAVES && defined(SHADOW_RES)"), Ok(1));
        assert_eq!(eval("!defined NOPE"), Ok(1));
        assert_eq!(eval("NOPE == 0"), Ok(1));
        assert_eq!(eval("undefinedFeature"), Ok(0));
    }

    #[test]
    fn bad_conditions_say_why() {
        assert!(eval("1 / 0").is_err());
        assert!(eval("SHADOW_RES == 0.5").is_err());
        assert!(eval("(1 + 2").is_err());
        assert!(eval("1 2").is_err());
        assert!(eval("").is_err());
        assert!(eval("defined").is_err());
        assert!(eval("1 @ 2").is_err());
    }
}
