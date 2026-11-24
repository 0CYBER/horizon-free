//! Loops that end by themselves.
//!
//! A backend that must not hand a compiler an endless loop (`force_loop_bounding`) gives every
//! loop a counter of its own and one more exit. Each iteration then runs a few more
//! instructions, and the trip count of a loop such as `for (int i = 0; i < 9; i++)` is hidden
//! from the driver's compiler, which keeps as a loop what it would have unrolled. A loop that
//! [`is_finite`] needs neither: it ends within 2^32 iterations whatever its variables hold.
//!
//! This module is Horizon's (THIRD_PARTY_NOTICES.md); only the SPIR-V backend asks it.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{
    BinaryOperator, Block, Expression, Function, Handle, Literal, LocalVariable, MathFunction,
    Module, Scalar, Statement, TypeInner, UnaryOperator,
};

static ENABLED: AtomicBool = AtomicBool::new(true);

/// Whether the backends ask [`is_finite`] at all: they do unless [`set_enabled`] said otherwise.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// With `false`, every module written from now on has all its loops bounded again, finite or
/// not, as the backends did before this module: for comparing the two.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// How far [`Counted::range`] follows an expression's operands and the values stored to the
/// variables it reads, before it takes a value as unknown. That also ends the walk through a
/// variable that is set from itself.
const DEPTH: u32 = 4;

/// How deep [`Counted::bounds`] follows the `!`, `&&` and `||` of a loop's test. An operand may
/// be used by several of them, so a test made of ever the same operand would be walked 2^depth
/// times: no test anyone writes nests this deep.
const NESTING: u32 = 16;

/// Whether the loop of `function` with this `body` and `continuing` block counts its way out,
/// whatever its variables hold:
///
/// - one of the statements its body always runs is an `if` that leaves the loop unless a local
///   `i32` or `u32` variable, read in that iteration, is below (or above) a bound, and nothing
///   before that `if` can `continue`;
/// - the loop stores to that variable in one place, which every iteration that goes on runs
///   once: the variable, read in that iteration, plus (or minus) a constant that moves it
///   toward the bound;
/// - nothing else in the loop can change the variable (no other store, no call given its
///   pointer); and
/// - a step past the bound can't wrap around: the step is 1 and the comparison strict, or the
///   bound is known to stay far enough from the end of the variable's type.
///
/// The bound may change as the loop runs. With a step of 1 and a strict comparison the variable
/// comes to the end of its type, where no bound lets it through, within 2^32 iterations; with
/// any other step or comparison, the bound's known range keeps the variable from wrapping, so
/// it passes the bound as soon.
pub fn is_finite(module: &Module, function: &Function, body: &Block, continuing: &Block) -> bool {
    let mut cx = Counted {
        module,
        function,
        body: Vec::new(),
        continuing: Vec::new(),
    };
    straight(body, &mut cx.body);
    straight(continuing, &mut cx.continuing);
    for (at, &statement) in cx.body.iter().enumerate() {
        if let Statement::If {
            condition,
            ref accept,
            ref reject,
        } = *statement
        {
            // The loop goes on past this `if` only while `condition` is false, or only while
            // it is true.
            let holds = if leaves(accept) {
                Some(false)
            } else if leaves(reject) {
                Some(true)
            } else {
                None
            };
            if holds.is_some_and(|holds| cx.bounds(at, condition, holds, 0)) {
                return true;
            }
        }
        // An iteration that `continue`s here goes on without meeting the `if`s further down.
        if skips(statement) {
            return false;
        }
    }
    false
}

/// Adds to `out` the statements `block` always runs, one after another: its own and those of
/// the blocks nested in it, but not those of its `if`s, `switch`es and loops.
fn straight<'a>(block: &'a [Statement], out: &mut Vec<&'a Statement>) {
    for statement in block {
        match *statement {
            Statement::Block(ref inner) => straight(inner, out),
            _ => out.push(statement),
        }
    }
}

/// Whether `block` leaves the loop it is directly in before it does anything else.
fn leaves(block: &Block) -> bool {
    match block.first() {
        Some(&Statement::Break | &Statement::Return { .. }) => true,
        Some(&Statement::Block(ref inner)) => leaves(inner),
        _ => false,
    }
}

/// Whether `statement` may `continue` the loop it is directly in.
fn skips(statement: &Statement) -> bool {
    match *statement {
        Statement::Continue => true,
        Statement::Block(ref block) => block.iter().any(skips),
        Statement::If {
            ref accept,
            ref reject,
            ..
        } => accept.iter().any(skips) || reject.iter().any(skips),
        Statement::Switch { ref cases, .. } => cases.iter().any(|case| case.body.iter().any(skips)),
        // A loop nested in it keeps its own `continue`s.
        _ => false,
    }
}

/// Whether one of `statements` (not a statement nested in them) emits `expr`.
fn emitted(statements: &[&Statement], expr: Handle<Expression>) -> bool {
    let index = expr.index() as u32;
    statements.iter().any(|statement| match **statement {
        Statement::Emit(ref range) => range.index_range().contains(&index),
        _ => false,
    })
}

/// Whether `expr` is the pointer to `var`.
fn points_to(function: &Function, expr: Handle<Expression>, var: Handle<LocalVariable>) -> bool {
    match function.expressions[expr] {
        Expression::LocalVariable(local) => local == var,
        _ => false,
    }
}

/// Where `statements` (not a statement nested in them) store to `var`.
fn store_at(
    function: &Function,
    statements: &[&Statement],
    var: Handle<LocalVariable>,
) -> Option<usize> {
    statements.iter().position(|statement| match **statement {
        Statement::Store { pointer, .. } => points_to(function, pointer, var),
        _ => false,
    })
}

/// Adds to `values` what the statements of `block`, nested ones too, store to `var`. False when
/// one of them may set `var` some other way.
fn stored_in(
    function: &Function,
    block: &[Statement],
    var: Handle<LocalVariable>,
    values: &mut Vec<Handle<Expression>>,
) -> bool {
    block
        .iter()
        .all(|statement| stored(function, statement, var, values))
}

/// [`stored_in`] for one statement. A call given the pointer to `var` may set it, and so may a
/// statement not listed here, for all this knows.
fn stored(
    function: &Function,
    statement: &Statement,
    var: Handle<LocalVariable>,
    values: &mut Vec<Handle<Expression>>,
) -> bool {
    match *statement {
        Statement::Emit(_)
        | Statement::Break
        | Statement::Continue
        | Statement::Return { .. }
        | Statement::Kill
        | Statement::ImageStore { .. } => true,
        Statement::Block(ref block) => stored_in(function, block, var, values),
        Statement::If {
            ref accept,
            ref reject,
            ..
        } => stored_in(function, accept, var, values) && stored_in(function, reject, var, values),
        Statement::Switch { ref cases, .. } => cases
            .iter()
            .all(|case| stored_in(function, &case.body, var, values)),
        Statement::Loop {
            ref body,
            ref continuing,
            ..
        } => stored_in(function, body, var, values) && stored_in(function, continuing, var, values),
        Statement::Store { pointer, value } => {
            if points_to(function, pointer, var) {
                values.push(value);
            }
            true
        }
        Statement::Call { ref arguments, .. } => !arguments
            .iter()
            .any(|&argument| points_to(function, argument, var)),
        _ => false,
    }
}

/// A loop [`is_finite`] is asked about.
struct Counted<'a> {
    module: &'a Module,
    function: &'a Function,
    /// The statements its body always runs, unless one of them leaves it ([`straight`]).
    body: Vec<&'a Statement>,
    /// The same of its `continuing` block, which every iteration that goes on runs to the end.
    continuing: Vec<&'a Statement>,
}

impl Counted<'_> {
    /// Whether `expr`, which is `holds` in every iteration that goes on past the body's
    /// statement `at`, keeps a counter of the loop from running forever. `depth` is how many
    /// `!`, `&&` and `||` it lies under in that statement's test ([`NESTING`]).
    fn bounds(&self, at: usize, expr: Handle<Expression>, holds: bool, depth: u32) -> bool {
        use BinaryOperator as Op;
        if depth > NESTING {
            return false;
        }
        let under = depth + 1;
        match self.function.expressions[expr] {
            Expression::Unary {
                op: UnaryOperator::LogicalNot,
                expr,
            } => self.bounds(at, expr, !holds, under),
            // Each operand of a true `&&` is true, and each operand of a false `||` is false.
            Expression::Binary {
                op: Op::LogicalAnd,
                left,
                right,
            } if holds => self.bounds(at, left, true, under) || self.bounds(at, right, true, under),
            Expression::Binary {
                op: Op::LogicalOr,
                left,
                right,
            } if !holds => {
                self.bounds(at, left, false, under) || self.bounds(at, right, false, under)
            }
            Expression::Binary { op, left, right } => {
                // Whether `left` is below `right` (else above it), and never equal to it.
                let (below, strict) = match op {
                    Op::Less => (true, true),
                    Op::LessEqual => (true, false),
                    Op::Greater => (false, true),
                    Op::GreaterEqual => (false, false),
                    _ => return false,
                };
                // A false comparison holds the other way round: `!(a < b)` is `a >= b`.
                let (below, strict) = if holds {
                    (below, strict)
                } else {
                    (!below, !strict)
                };
                self.counts(at, left, below, strict, right)
                    || self.counts(at, right, !below, strict, left)
            }
            _ => false,
        }
    }

    /// Whether `load` reads a counter of the loop that cannot run forever while it stays below
    /// `bound` (`below`; else above it), or at it unless `strict`, as it does in every
    /// iteration that goes on past the body's statement `at`.
    fn counts(
        &self,
        at: usize,
        load: Handle<Expression>,
        below: bool,
        strict: bool,
        bound: Handle<Expression>,
    ) -> bool {
        let Some((var, full)) = self.loaded(load) else {
            return false;
        };
        // The comparison is about this iteration's value, and each iteration steps toward the
        // bound.
        if !emitted(&self.body[..at], load) {
            return false;
        }
        let Some(step) = self.step(var) else {
            return false;
        };
        if step == 0 || (step > 0) != below {
            return false;
        }
        // How far from the end of its type the bound must stay, for the step that takes the
        // counter past the bound not to wrap around: nowhere, for a step of 1 past a bound the
        // counter never equals.
        let room = step.abs() - 1 + i64::from(!strict);
        if room == 0 {
            return true;
        }
        let (low, high) = self.range(bound, full, 0);
        if below {
            high <= full.1 - room
        } else {
            low >= full.0 + room
        }
    }

    /// The local `i32` or `u32` variable `expr` reads, with the lowest and highest value of its
    /// type.
    fn loaded(&self, expr: Handle<Expression>) -> Option<(Handle<LocalVariable>, (i64, i64))> {
        let Expression::Load { pointer } = self.function.expressions[expr] else {
            return None;
        };
        let Expression::LocalVariable(var) = self.function.expressions[pointer] else {
            return None;
        };
        match self.module.types[self.function.local_variables[var].ty].inner {
            TypeInner::Scalar(Scalar::I32) => {
                Some((var, (i64::from(i32::MIN), i64::from(i32::MAX))))
            }
            TypeInner::Scalar(Scalar::U32) => Some((var, (0, i64::from(u32::MAX)))),
            _ => None,
        }
    }

    /// What every iteration that goes on adds to `var`: the constant that the loop's one store
    /// to `var` adds to the value read in that iteration.
    fn step(&self, var: Handle<LocalVariable>) -> Option<i64> {
        let mut values = Vec::new();
        for &statement in self.body.iter().chain(&self.continuing) {
            if !stored(self.function, statement, var, &mut values) {
                return None;
            }
        }
        if values.len() != 1 {
            return None;
        }
        // That store is one the `continuing` block always runs, or one the body always runs
        // with no `continue` before it.
        let (list, at) = match store_at(self.function, &self.continuing, var) {
            Some(at) => (&self.continuing, at),
            None => {
                let at = store_at(self.function, &self.body, var)?;
                if self.body[..at].iter().any(|&statement| skips(statement)) {
                    return None;
                }
                (&self.body, at)
            }
        };
        let Expression::Binary { op, left, right } = self.function.expressions[values[0]] else {
            return None;
        };
        // What it adds to is this iteration's value: read by a statement before the store.
        let read = |expr: Handle<Expression>| {
            self.loaded(expr).is_some_and(|(loaded, _)| loaded == var) && emitted(&list[..at], expr)
        };
        match op {
            BinaryOperator::Add if read(left) => self.constant(right),
            BinaryOperator::Add if read(right) => self.constant(left),
            BinaryOperator::Subtract if read(left) => self.constant(right).map(|by| -by),
            _ => None,
        }
    }

    /// The value of `expr` when it is an `i32` or `u32` literal, or a constant that is one.
    fn constant(&self, expr: Handle<Expression>) -> Option<i64> {
        let literal = match self.function.expressions[expr] {
            Expression::Literal(literal) => literal,
            Expression::Constant(constant) => {
                match self.module.global_expressions[self.module.constants[constant].init] {
                    Expression::Literal(literal) => literal,
                    _ => return None,
                }
            }
            _ => return None,
        };
        match literal {
            Literal::I32(value) => Some(i64::from(value)),
            Literal::U32(value) => Some(i64::from(value)),
            _ => None,
        }
    }

    /// The lowest and highest value `expr` can have whenever it is evaluated: `full`, the ends
    /// of its type, where this can't tell.
    fn range(&self, expr: Handle<Expression>, full: (i64, i64), depth: u32) -> (i64, i64) {
        if let Some(value) = self.constant(expr) {
            return (value, value);
        }
        if depth == DEPTH {
            return full;
        }
        let range = |expr: Handle<Expression>| self.range(expr, full, depth + 1);
        let (low, high) = match self.function.expressions[expr] {
            // All a variable ever holds: what it starts with, and what the function stores to
            // it.
            Expression::Load { .. } => {
                let Some((var, _)) = self.loaded(expr) else {
                    return full;
                };
                let mut values = Vec::new();
                if !stored_in(self.function, &self.function.body, var, &mut values) {
                    return full;
                }
                let mut all = match self.function.local_variables[var].init {
                    Some(init) => range(init),
                    None => (0, 0),
                };
                for value in values {
                    let other = range(value);
                    all = (all.0.min(other.0), all.1.max(other.1));
                }
                all
            }
            Expression::Select { accept, reject, .. } => {
                let (a, b) = (range(accept), range(reject));
                (a.0.min(b.0), a.1.max(b.1))
            }
            Expression::Math {
                fun: MathFunction::Min,
                arg,
                arg1: Some(other),
                ..
            } => {
                let (a, b) = (range(arg), range(other));
                (a.0.min(b.0), a.1.min(b.1))
            }
            Expression::Math {
                fun: MathFunction::Max,
                arg,
                arg1: Some(other),
                ..
            } => {
                let (a, b) = (range(arg), range(other));
                (a.0.max(b.0), a.1.max(b.1))
            }
            Expression::Binary {
                op: BinaryOperator::Add,
                left,
                right,
            } => {
                let (a, b) = (range(left), range(right));
                (a.0 + b.0, a.1 + b.1)
            }
            Expression::Binary {
                op: BinaryOperator::Subtract,
                left,
                right,
            } => {
                let (a, b) = (range(left), range(right));
                (a.0 - b.1, a.1 - b.0)
            }
            _ => return full,
        };
        // A sum past either end of the type wraps around.
        if low < full.0 || high > full.1 {
            full
        } else {
            (low, high)
        }
    }
}
