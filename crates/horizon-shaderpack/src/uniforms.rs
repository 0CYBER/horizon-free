//! Custom uniforms: `uniform.<type>.<name> = <expression>` and
//! `variable.<type>.<name> = <expression>` in `shaders.properties`, evaluated
//! once a frame the way Iris does (`CustomUniforms`, the `kroppeb.stareval`
//! parser, `IrisFunctions`, `SmoothFloat`).
//!
//! Values are lists of floats: one for `bool`, `int` and `float` (a bool is 0
//! or 1), two to four for vectors. Iris types each expression; evaluating in
//! floats gives the same results for what packs write (its only integer
//! operations are `+ - * %`, exact in floats at these sizes; `/` is always a
//! float division).
//!
//! A definition Iris can't resolve (a syntax error, an unknown name or
//! function, a cycle) is left out, and so is every definition using it: the
//! shader's uniform keeps its default, zero. Here a name is unknown when it is
//! neither another definition nor a value the `builtin` lookup gives.

use std::collections::HashMap;

/// The custom uniforms and variables of a pack, in evaluation order.
#[derive(Clone, Debug, Default)]
pub struct CustomUniforms {
    defs: Vec<Def>,
    /// Evaluation order (indices into `defs`): dependencies first.
    order: Vec<usize>,
    /// What couldn't be parsed or resolved, for the log.
    pub problems: Vec<String>,
}

#[derive(Clone, Debug)]
struct Def {
    name: String,
    /// Components of the result: 1, 2, 3 or 4.
    size: usize,
    /// `bool` (0 or 1) or `int` (truncated) rather than float.
    kind: Kind,
    /// `uniform.` (given to the shaders) rather than `variable.`.
    uniform: bool,
    expr: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Bool,
    Int,
    Float,
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Number(f32),
    Name(String),
    /// A vector's component (`.x`, `.g`, `.t`, `.2`...).
    Access(Box<Expr>, usize),
    Not(Box<Expr>),
    Negate(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    /// A function; `smooth` calls carry their own state slot.
    Call(String, Vec<Expr>, usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Multiply,
    Divide,
    Remainder,
    Add,
    Subtract,
    Equals,
    NotEquals,
    Less,
    More,
    LessOrEqual,
    MoreOrEqual,
    And,
    Or,
}

impl Op {
    /// `IrisOptions`: lower binds tighter; equal priorities associate left.
    fn priority(self) -> u8 {
        match self {
            Op::Multiply | Op::Divide | Op::Remainder => 0,
            Op::Add | Op::Subtract => 1,
            Op::Equals
            | Op::NotEquals
            | Op::Less
            | Op::More
            | Op::LessOrEqual
            | Op::MoreOrEqual => 2,
            Op::And | Op::Or => 3,
        }
    }
}

/// The per-frame state: `smooth` accumulators, `random`'s generator and the
/// last values.
#[derive(Clone, Debug, Default)]
pub struct UniformState {
    smooth: Vec<SmoothFloat>,
    seed: u64,
    values: HashMap<String, Vec<f32>>,
}

impl UniformState {
    /// A uniform's value this frame; `None` for variables and for
    /// definitions that couldn't be resolved.
    pub fn get(&self, name: &str) -> Option<&[f32]> {
        self.values.get(name).map(Vec::as_slice)
    }

    /// Every uniform's value this frame, by name.
    pub fn values(&self) -> &HashMap<String, Vec<f32>> {
        &self.values
    }
}

impl CustomUniforms {
    /// The definitions in `properties` (key, value), in file order.
    pub fn parse<'a>(properties: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut out = Self::default();
        let mut smooth = 0;
        for (key, value) in properties {
            let (uniform, rest) = if let Some(rest) = key.strip_prefix("uniform.") {
                (true, rest)
            } else if let Some(rest) = key.strip_prefix("variable.") {
                (false, rest)
            } else {
                continue;
            };
            let Some((ty, name)) = rest.split_once('.') else {
                continue;
            };
            let (kind, size) = match ty {
                "bool" => (Kind::Bool, 1),
                "int" => (Kind::Int, 1),
                "float" => (Kind::Float, 1),
                "vec2" => (Kind::Float, 2),
                "vec3" => (Kind::Float, 3),
                "vec4" => (Kind::Float, 4),
                _ => {
                    out.problems.push(format!("{key}: unknown type `{ty}`"));
                    continue;
                }
            };
            if out.defs.iter().any(|d| d.name == name) {
                out.problems.push(format!("{key}: duplicated variable"));
                continue;
            }
            match parse(value, &mut smooth) {
                Ok(expr) => out.defs.push(Def {
                    name: name.to_owned(),
                    size,
                    kind,
                    uniform,
                    expr,
                }),
                Err(e) => out.problems.push(format!("{key}: {e} ( = {value})")),
            }
        }
        out.order = out.sort();
        out
    }

    /// Whether the pack defines none.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Dependencies first; a definition in a cycle is left out.
    fn sort(&mut self) -> Vec<usize> {
        let index: HashMap<&str, usize> = self
            .defs
            .iter()
            .enumerate()
            .map(|(i, d)| (d.name.as_str(), i))
            .collect();
        // 0 unvisited, 1 in progress, 2 done, 3 broken.
        let mut mark = vec![0u8; self.defs.len()];
        let mut order = Vec::new();
        fn visit(
            i: usize,
            defs: &[Def],
            index: &HashMap<&str, usize>,
            mark: &mut [u8],
            order: &mut Vec<usize>,
        ) -> bool {
            match mark[i] {
                1 | 3 => return false,
                2 => return true,
                _ => {}
            }
            mark[i] = 1;
            let mut names = Vec::new();
            defs[i].expr.names(&mut names);
            let ok = names.iter().all(|n| match index.get(n.as_str()) {
                Some(&j) => visit(j, defs, index, mark, order),
                None => true,
            });
            mark[i] = if ok { 2 } else { 3 };
            if ok {
                order.push(i);
            }
            ok
        }
        for i in 0..self.defs.len() {
            if !visit(i, &self.defs, &index, &mut mark, &mut order) && mark[i] == 3 {
                self.problems
                    .push(format!("{}: circular reference", self.defs[i].name));
            }
        }
        order
    }

    /// Evaluate every definition for this frame: `builtin` gives Iris's own
    /// uniforms, `frame_time` (seconds) drives `smooth`.
    pub fn update(
        &self,
        state: &mut UniformState,
        frame_time: f32,
        builtin: &dyn Fn(&str) -> Option<Vec<f32>>,
    ) {
        let mut values: HashMap<&str, Vec<f32>> = HashMap::new();
        let mut broken: Vec<&str> = Vec::new();
        state.values.clear();
        for &i in &self.order {
            let def = &self.defs[i];
            let mut eval = Eval {
                frame_time,
                builtin,
                values: &values,
                broken: &broken,
                smooth: &mut state.smooth,
                seed: &mut state.seed,
            };
            let Some(mut value) = eval.expr(&def.expr) else {
                broken.push(&def.name);
                continue;
            };
            value.resize(def.size, value.last().copied().unwrap_or(0.0));
            match def.kind {
                Kind::Bool => value[0] = f32::from(u8::from(value[0] != 0.0)),
                Kind::Int => value[0] = value[0].trunc(),
                Kind::Float => {}
            }
            if def.uniform {
                state.values.insert(def.name.clone(), value.clone());
            }
            values.insert(&def.name, value);
        }
    }
}

impl Expr {
    fn names(&self, out: &mut Vec<String>) {
        match self {
            Expr::Number(_) => {}
            Expr::Name(n) => out.push(n.clone()),
            Expr::Access(e, _) | Expr::Not(e) | Expr::Negate(e) => e.names(out),
            Expr::Binary(_, a, b) => {
                a.names(out);
                b.names(out);
            }
            Expr::Call(_, args, _) => args.iter().for_each(|a| a.names(out)),
        }
    }
}

struct Eval<'a> {
    frame_time: f32,
    builtin: &'a dyn Fn(&str) -> Option<Vec<f32>>,
    values: &'a HashMap<&'a str, Vec<f32>>,
    broken: &'a [&'a str],
    smooth: &'a mut Vec<SmoothFloat>,
    seed: &'a mut u64,
}

/// Apply `f` component by component, a scalar standing for every component.
fn zip(args: &[Vec<f32>], f: impl Fn(&[f32]) -> f32) -> Vec<f32> {
    let n = args.iter().map(Vec::len).max().unwrap_or(1);
    (0..n)
        .map(|i| {
            let a: Vec<f32> = args
                .iter()
                .map(|v| {
                    if v.len() == 1 {
                        v[0]
                    } else {
                        v.get(i).copied().unwrap_or(0.0)
                    }
                })
                .collect();
            f(&a)
        })
        .collect()
}

fn truth(b: bool) -> f32 {
    f32::from(u8::from(b))
}

impl Eval<'_> {
    fn expr(&mut self, e: &Expr) -> Option<Vec<f32>> {
        Some(match e {
            Expr::Number(n) => vec![*n],
            Expr::Name(n) => self.name(n)?,
            Expr::Access(e, i) => vec![*self.expr(e)?.get(*i)?],
            Expr::Not(e) => zip(&[self.expr(e)?], |a| truth(a[0] == 0.0)),
            Expr::Negate(e) => self.expr(e)?.iter().map(|v| -v).collect(),
            Expr::Binary(op, a, b) => {
                let a = self.expr(a)?;
                let b = self.expr(b)?;
                let f = |x: f32, y: f32| match op {
                    Op::Multiply => x * y,
                    Op::Divide => x / y,
                    // Java's `%`, on ints and floats: the sign of x.
                    Op::Remainder => x % y,
                    Op::Add => x + y,
                    Op::Subtract => x - y,
                    Op::Equals => truth(x == y),
                    Op::NotEquals => truth(x != y),
                    Op::Less => truth(x < y),
                    Op::More => truth(x > y),
                    Op::LessOrEqual => truth(x <= y),
                    Op::MoreOrEqual => truth(x >= y),
                    Op::And => truth(x != 0.0 && y != 0.0),
                    Op::Or => truth(x != 0.0 || y != 0.0),
                };
                zip(&[a, b], |v| f(v[0], v[1]))
            }
            Expr::Call(name, args, slot) => self.call(name, args, *slot)?,
        })
    }

    fn name(&self, n: &str) -> Option<Vec<f32>> {
        if self.broken.contains(&n) {
            return None;
        }
        if let Some(v) = self.values.get(n) {
            return Some(v.clone());
        }
        match n {
            "true" => return Some(vec![1.0]),
            "false" => return Some(vec![0.0]),
            "pi" => return Some(vec![std::f32::consts::PI]),
            _ => {}
        }
        (self.builtin)(n)
    }

    fn random(&mut self) -> f32 {
        // xorshift64*, seeded once.
        if *self.seed == 0 {
            *self.seed = 0x9E37_79B9_7F4A_7C15;
        }
        let mut x = *self.seed;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        *self.seed = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }

    fn call(&mut self, name: &str, args: &[Expr], slot: usize) -> Option<Vec<f32>> {
        // `if` evaluates only the branch it takes.
        if name == "if" {
            if args.len() < 3 || args.len() % 2 == 0 {
                return None;
            }
            for pair in args[..args.len() - 1].chunks(2) {
                if self.expr(&pair[0])?[0] != 0.0 {
                    return self.expr(&pair[1]);
                }
            }
            return self.expr(&args[args.len() - 1]);
        }
        if name == "smooth" {
            return self.smooth(args, slot);
        }
        let v: Vec<Vec<f32>> = args.iter().map(|a| self.expr(a)).collect::<Option<_>>()?;
        let unary = |f: fn(f32) -> f32| {
            (v.len() == 1).then(|| v[0].iter().map(|&x| f(x)).collect::<Vec<f32>>())
        };
        let n = v.len();
        Some(match (name, n) {
            ("sin", 1) => unary(f32::sin)?,
            ("cos", 1) => unary(f32::cos)?,
            ("tan", 1) => unary(f32::tan)?,
            ("asin", 1) => unary(f32::asin)?,
            ("acos", 1) => unary(f32::acos)?,
            ("atan", 1) => unary(f32::atan)?,
            ("atan" | "atan2", 2) => zip(&v, |a| a[0].atan2(a[1])),
            ("torad" | "radians", 1) => unary(f32::to_radians)?,
            ("todeg" | "degrees", 1) => unary(f32::to_degrees)?,
            ("pow", 2) => zip(&v, |a| a[0].powf(a[1])),
            ("exp", 1) => unary(f32::exp)?,
            ("exp2", 1) => unary(f32::exp2)?,
            ("exp10", 1) => unary(|x| 10f32.powf(x))?,
            ("log", 1) => unary(f32::ln)?,
            ("log", 2) => zip(&v, |a| a[1].ln() / a[0].ln()),
            ("log2", 1) => unary(f32::log2)?,
            ("log10", 1) => unary(f32::log10)?,
            ("sqrt", 1) => unary(f32::sqrt)?,
            ("abs", 1) => unary(f32::abs)?,
            // Math.signum: 0 stays 0.
            ("sign" | "signum", 1) => unary(|x| if x == 0.0 { x } else { x.signum() })?,
            ("floor", 1) => unary(f32::floor)?,
            ("ceil", 1) => unary(f32::ceil)?,
            ("frac", 1) => unary(|x| x - x.floor())?,
            // Math.round: half up.
            ("round", 1) => unary(|x| (x + 0.5).floor())?,
            ("min", 2..) => zip(&v, |a| a.iter().copied().fold(f32::INFINITY, f32::min)),
            ("max", 2..) => zip(&v, |a| a.iter().copied().fold(f32::NEG_INFINITY, f32::max)),
            ("clamp", 3) => zip(&v, |a| a[0].max(a[1]).min(a[2])),
            ("mix", 3) => zip(&v, |a| a[0] + (a[1] - a[0]) * a[2]),
            ("edge", 2) => zip(&v, |a| truth(a[1] >= a[0])),
            ("fmod", 2) => zip(&v, |a| (a[0] % a[1] + a[1]) % a[1]),
            ("between", 3) => vec![truth(v[0][0] >= v[1][0] && v[0][0] <= v[2][0])],
            ("equals", 3) => vec![truth((v[0][0] - v[1][0]).abs() <= v[2][0])],
            ("in", 2..) => vec![truth(v[1..].iter().any(|x| x[0] == v[0][0]))],
            ("random", 0) => vec![self.random()],
            ("random", 2) => {
                let r = self.random();
                vec![v[0][0] + r * (v[1][0] - v[0][0])]
            }
            ("vec2" | "ivec2" | "bvec2", 2)
            | ("vec3" | "ivec3" | "bvec3", 3)
            | ("vec4" | "ivec4" | "bvec4", 4) => {
                let mut out: Vec<f32> = v.iter().map(|a| a[0]).collect();
                if name.starts_with('i') {
                    out.iter_mut().for_each(|x| *x = x.trunc());
                } else if name.starts_with('b') {
                    out.iter_mut().for_each(|x| *x = truth(*x != 0.0));
                }
                out
            }
            _ => return None,
        })
    }

    /// `smooth([id,] value[, fadeTime[, fadeDownTime]])`: the half-lives, in
    /// tenths of a second, default to 1. A constant first argument of two or
    /// three is OptiFine's id; with four it always is.
    fn smooth(&mut self, args: &[Expr], slot: usize) -> Option<Vec<f32>> {
        let id = matches!(args.first(), Some(Expr::Number(_))) && args.len() >= 2;
        let args = if id || args.len() == 4 {
            &args[1..]
        } else {
            args
        };
        let value = self.expr(args.first()?)?[0];
        let up = match args.get(1) {
            Some(e) => self.expr(e)?[0],
            None => 1.0,
        };
        let down = match args.get(2) {
            Some(e) => self.expr(e)?[0],
            None => up,
        };
        if args.len() > 3 {
            return None;
        }
        if self.smooth.len() <= slot {
            self.smooth.resize(slot + 1, SmoothFloat::default());
        }
        Some(vec![self.smooth[slot].update(
            value,
            up,
            down,
            self.frame_time,
        )])
    }
}

/// Iris's `SmoothFloat`: exponential smoothing with separate half-lives up
/// and down, in tenths of a second; the first value is taken as is.
#[derive(Clone, Copy, Debug, Default)]
pub struct SmoothFloat {
    value: Option<f32>,
}

impl SmoothFloat {
    pub fn update(&mut self, target: f32, half_life_up: f32, half_life_down: f32, dt: f32) -> f32 {
        let Some(acc) = self.value else {
            self.value = Some(target);
            return target;
        };
        let half_life = if target > acc {
            half_life_up
        } else {
            half_life_down
        };
        let next = if half_life == 0.0 {
            target
        } else {
            let decay = std::f32::consts::LN_2 / (half_life * 0.1);
            let alpha = 1.0 - (-decay * dt).exp();
            (1.0 - alpha) * acc + alpha * target
        };
        self.value = Some(next);
        next
    }
}

/// Parse one expression; `smooth` counts the `smooth` calls so far (each
/// keeps its own state).
fn parse(text: &str, smooth: &mut usize) -> Result<Expr, String> {
    let tokens = tokenize(text)?;
    let mut p = Parser {
        tokens,
        at: 0,
        smooth,
    };
    let e = p.binary(3)?;
    match p.tokens.get(p.at) {
        None => Ok(e),
        Some(t) => Err(format!("unexpected {t:?}")),
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f32),
    Id(String),
    /// `.` followed by an accessor.
    Access(String),
    Op(String),
    Open,
    Close,
    Comma,
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let read_while = |i: &mut usize, f: &dyn Fn(char) -> bool| {
        let start = *i;
        while *i < chars.len() && f(chars[*i]) {
            *i += 1;
        }
        chars[start..*i].iter().collect::<String>()
    };
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_alphabetic() || c == '_' {
            out.push(Token::Id(read_while(&mut i, &|c| {
                c.is_ascii_alphanumeric() || c == '_'
            })));
        } else if c == '.'
            && matches!(
                out.last(),
                Some(Token::Id(_) | Token::Close | Token::Access(_))
            )
        {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let access = read_while(&mut i, &|c| c.is_ascii_alphanumeric() || c == '_');
            if access.is_empty() {
                return Err("an expression can't end with `.`".into());
            }
            out.push(Token::Access(access));
        } else if c.is_ascii_digit() || c == '.' {
            let s = read_while(&mut i, &|c| c.is_ascii_alphanumeric() || c == '.');
            let n = s
                .parse::<f32>()
                .or_else(|_| s.trim_end_matches(['f', 'F']).parse::<f32>())
                .map_err(|_| format!("bad number `{s}`"))?;
            out.push(Token::Number(n));
        } else if c == '(' {
            out.push(Token::Open);
            i += 1;
        } else if c == ')' {
            out.push(Token::Close);
            i += 1;
        } else if c == ',' {
            out.push(Token::Comma);
            i += 1;
        } else {
            let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
            let op = ["==", "!=", "<=", ">=", "&&", "||"]
                .into_iter()
                .find(|o| two == *o)
                .map(str::to_owned)
                .or_else(|| "*/%+-<>!≠≤≥".contains(c).then(|| c.to_string()))
                .ok_or_else(|| format!("unexpected character `{c}`"))?;
            i += op.chars().count();
            out.push(Token::Op(op));
        }
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Token>,
    at: usize,
    smooth: &'a mut usize,
}

impl Parser<'_> {
    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn peek_op(&self) -> Option<Op> {
        let Some(Token::Op(o)) = self.tokens.get(self.at) else {
            return None;
        };
        Some(match o.as_str() {
            "*" => Op::Multiply,
            "/" => Op::Divide,
            "%" => Op::Remainder,
            "+" => Op::Add,
            "-" => Op::Subtract,
            "==" => Op::Equals,
            "!=" | "≠" => Op::NotEquals,
            "<" => Op::Less,
            ">" => Op::More,
            "<=" | "≤" => Op::LessOrEqual,
            ">=" | "≥" => Op::MoreOrEqual,
            "&&" => Op::And,
            "||" => Op::Or,
            _ => return None,
        })
    }

    /// Operators of priority `max` or tighter, left to right.
    fn binary(&mut self, max: u8) -> Result<Expr, String> {
        let mut left = self.unary()?;
        while let Some(op) = self.peek_op() {
            let p = op.priority();
            if p > max {
                break;
            }
            self.at += 1;
            // The right side takes only tighter operators: equal priorities
            // associate to the left.
            let right = if p == 0 {
                self.unary()?
            } else {
                self.binary(p - 1)?
            };
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Prefix `!` and `-`, which bind tighter than any binary operator.
    fn unary(&mut self) -> Result<Expr, String> {
        match self.tokens.get(self.at) {
            Some(Token::Op(o)) if o == "!" => {
                self.at += 1;
                Ok(Expr::Not(Box::new(self.unary()?)))
            }
            Some(Token::Op(o)) if o == "-" => {
                self.at += 1;
                Ok(Expr::Negate(Box::new(self.unary()?)))
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let mut e = match self.next() {
            Some(Token::Number(n)) => Expr::Number(n),
            Some(Token::Id(id)) if self.tokens.get(self.at) == Some(&Token::Open) => {
                self.at += 1;
                let mut args = Vec::new();
                if self.tokens.get(self.at) == Some(&Token::Close) {
                    self.at += 1;
                } else {
                    loop {
                        args.push(self.binary(3)?);
                        match self.next() {
                            Some(Token::Comma) => {}
                            Some(Token::Close) => break,
                            t => return Err(format!("expected `,` or `)`, found {t:?}")),
                        }
                    }
                }
                let slot = *self.smooth;
                if id == "smooth" {
                    *self.smooth += 1;
                }
                Expr::Call(id, args, slot)
            }
            Some(Token::Id(id)) => Expr::Name(id),
            Some(Token::Open) => {
                let e = self.binary(3)?;
                match self.next() {
                    Some(Token::Close) => e,
                    t => return Err(format!("expected `)`, found {t:?}")),
                }
            }
            t => return Err(format!("expected a value, found {t:?}")),
        };
        while let Some(Token::Access(a)) = self.tokens.get(self.at) {
            let i = match a.as_str() {
                "x" | "r" | "s" | "0" => 0,
                "y" | "g" | "t" | "1" => 1,
                "z" | "b" | "p" | "2" => 2,
                "w" | "a" | "q" | "3" => 3,
                _ => return Err(format!("unknown accessor `.{a}`")),
            };
            self.at += 1;
            e = Expr::Access(Box::new(e), i);
        }
        Ok(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtins(name: &str) -> Option<Vec<f32>> {
        Some(match name {
            "frameCounter" => vec![7.0],
            "eyeBrightness" => vec![32.0, 120.0],
            "isEyeInWater" => vec![0.0],
            "eyeAltitude" => vec![40.0],
            "cameraPosition" => vec![1.0, 64.5, -3.0],
            "rainStrength" => vec![0.0],
            _ => return None,
        })
    }

    fn eval(text: &str) -> Option<Vec<f32>> {
        let u = CustomUniforms::parse([("uniform.float.x", text)]);
        let mut state = UniformState::default();
        u.update(&mut state, 0.05, &builtins);
        state.get("x").map(<[f32]>::to_vec)
    }

    #[test]
    fn operators_follow_iris_priorities() {
        assert_eq!(eval("1 + 2 * 3"), Some(vec![7.0]));
        assert_eq!(eval("10 - 4 - 3"), Some(vec![3.0]));
        assert_eq!(eval("2 * 3 % 4"), Some(vec![2.0]));
        assert_eq!(eval("-2 * 3"), Some(vec![-6.0]));
        assert_eq!(eval("1 < 2 && 3 > 4 || 1"), Some(vec![1.0]));
        assert_eq!(eval("!(1 == 1)"), Some(vec![0.0]));
        assert_eq!(eval("frameCounter % 2"), Some(vec![1.0]));
        assert_eq!(eval("7 / 2"), Some(vec![3.5]));
        assert_eq!(eval("-7 % 4"), Some(vec![-3.0]));
        assert_eq!(eval("eyeBrightness.y / 240.0"), Some(vec![0.5]));
        assert_eq!(eval("cameraPosition.y - 0.5"), Some(vec![64.0]));
        assert_eq!(eval("2.5f * 2"), Some(vec![5.0]));
    }

    #[test]
    fn functions() {
        assert_eq!(eval("if(eyeBrightness.y > 239.0, 1, 0)"), Some(vec![0.0]));
        assert_eq!(eval("if(0, 1, 1 == 1, 2, 3)"), Some(vec![2.0]));
        assert_eq!(
            eval("clamp(5, 0, 1) + max(1, 4, 2) + min(3, -1)"),
            Some(vec![4.0])
        );
        assert_eq!(eval("fmod(-1, 4)"), Some(vec![3.0]));
        assert_eq!(eval("in(3, 1, 2, 3)"), Some(vec![1.0]));
        assert_eq!(
            eval("between(2, 1, 3) && equals(1.0, 1.05, 0.1)"),
            Some(vec![1.0])
        );
        assert_eq!(eval("frac(2.25) + floor(-0.5) + abs(-2)"), Some(vec![1.25]));
        assert_eq!(eval("vec2(1, 2).y"), Some(vec![2.0]));
        // An unknown name or function leaves the uniform out (zero).
        assert_eq!(eval("biome_precipitation + 1"), None);
        assert_eq!(eval("nope(1)"), None);
        assert_eq!(eval("1 +"), None);
    }

    #[test]
    fn variables_resolve_in_dependency_order() {
        let u = CustomUniforms::parse([
            ("uniform.float.b", "a * 2"),
            ("variable.float.a", "frameCounter + 1"),
            ("uniform.vec2.v", "vec2(a, b)"),
            ("uniform.bool.flag", "b > 10"),
            ("uniform.float.loop1", "loop2"),
            ("uniform.float.loop2", "loop1"),
            ("uniform.float.bad", "unknown"),
            ("uniform.float.uses_bad", "bad + 1"),
        ]);
        let mut state = UniformState::default();
        u.update(&mut state, 0.05, &builtins);
        assert_eq!(state.get("b"), Some(&[16.0][..]));
        assert_eq!(state.get("a"), None, "variables aren't uniforms");
        assert_eq!(state.get("v"), Some(&[8.0, 16.0][..]));
        assert_eq!(state.get("flag"), Some(&[1.0][..]));
        assert_eq!(state.get("loop1"), None);
        assert_eq!(state.get("bad"), None);
        assert_eq!(state.get("uses_bad"), None);
        assert!(u.problems.iter().any(|p| p.contains("circular")));
    }

    #[test]
    fn smooth_is_iris_exponential_smoothing() {
        // Complementary's `eyeBrightnessM` and `isEyeInCave`.
        let u = CustomUniforms::parse([
            (
                "uniform.float.m",
                "smooth(4, eyeBrightness.y / 240.0, 5, 5)",
            ),
            (
                "uniform.float.cave",
                "if(isEyeInWater == 0, 1.0 - smooth(202, if(eyeAltitude < 5.0, eyeBrightness.y / 240.0, 1.0), 6, 12), 0.0)",
            ),
            ("uniform.float.once", "smooth(frameCounter)"),
        ]);
        assert!(u.problems.is_empty(), "{:?}", u.problems);
        let mut state = UniformState::default();
        u.update(&mut state, 0.05, &builtins);
        // The first value isn't smoothed.
        assert_eq!(state.get("m"), Some(&[0.5][..]));
        assert_eq!(state.get("cave"), Some(&[0.0][..]));
        // Half a second at a half-life of 5 tenths: halfway to the target.
        let dark = |name: &str| -> Option<Vec<f32>> {
            match name {
                "eyeBrightness" => Some(vec![0.0, 0.0]),
                _ => builtins(name),
            }
        };
        u.update(&mut state, 0.5, &dark);
        let m = state.get("m").unwrap()[0];
        assert!((m - 0.25).abs() < 1e-5, "{m}");
        let mut s = SmoothFloat::default();
        s.update(0.0, 0.0, 10.0, 0.1);
        assert_eq!(s.update(1.0, 0.0, 10.0, 0.1), 1.0, "a half-life of 0 jumps");
        let down = s.update(0.0, 0.0, 10.0, 1.0);
        assert!((down - 0.5).abs() < 1e-5, "{down}");
    }
}
