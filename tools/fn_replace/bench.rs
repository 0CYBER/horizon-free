//! fn-replace bench: one game function, `horizon_client::game::autosave_interval(rate: f32) -> i64`,
//! called from outside the prebuilt game, built with rustc against the SDK folder alone.
//!
//! usage: bench <mode> [millions of calls]
//!   plain    the same body in this crate, never inlined: no replacement check
//!   idle     the game's function with nothing installed
//!   patched  the patched copy installed through the typed installer
extern crate horizon_mod_host;
use horizon_client::game as g;
use std::hint::black_box;
use std::time::Instant;

#[inline(never)]
fn plain(rate: f32) -> i64 {
    i64::from((rate * 300.0) as i32).max(100)
}

/// A mixin's listener inlined into the copy (MODIFY_EXPRESSION_VALUE doubling the value).
#[inline]
fn listener(value: i64) -> i64 {
    value * 2
}

/// The patched copy as function replacement compiles it: the function's own signature.
fn copy(rate: f32) -> i64 {
    listener(i64::from((rate * 300.0) as i32).max(100))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("mode");
    let calls: u64 = args.next().map_or(100, |n| n.parse().unwrap()) * 1_000_000;
    match mode.as_str() {
        "patched" => assert!(g::__fnr_autosave_interval(copy)),
        "plain" | "idle" => {}
        _ => panic!("expected plain, idle or patched"),
    }
    let f: fn(f32) -> i64 = if mode == "plain" {
        plain
    } else {
        g::autosave_interval
    };
    let check = f(1.0);
    let started = Instant::now();
    let mut sum = 0i64;
    for i in 0..calls {
        sum = sum.wrapping_add(f(black_box(20.0 + (i & 7) as f32)));
    }
    let elapsed = started.elapsed();
    println!(
        "{mode}: {:.2} ns/call (f(1.0) = {check}, sum {sum})",
        elapsed.as_nanos() as f64 / calls as f64
    );
}
