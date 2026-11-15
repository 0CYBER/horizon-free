// This same checker is compiled into the real wasm fixture; it never asks a
// native host to compute or echo math results.
#[path = "strict_math_vectors.rs"]
mod vectors;

fn exact(actual: f64, expected: u64, operation: &str, x: u64, y: Option<u64>) {
    if f64::from_bits(expected).is_nan() {
        assert!(actual.is_nan(), "{operation}({x:016x}, {y:?}) must be NaN");
    } else {
        assert_eq!(actual.to_bits(), expected, "{operation}({x:016x}, {y:?})");
    }
}

pub(crate) fn check_vectors() -> usize {
    use mod_api::strict_math;
    let mut checked = 0;
    for (name, operation, cases) in [
        ("sin", strict_math::sin as fn(f64) -> f64, vectors::SIN),
        ("cos", strict_math::cos, vectors::COS),
        ("tan", strict_math::tan, vectors::TAN),
        ("exp", strict_math::exp, vectors::EXP),
    ] {
        assert!(cases.len() >= 500);
        for &(x, expected) in cases {
            exact(operation(f64::from_bits(x)), expected, name, x, None);
            checked += 1;
        }
    }
    assert!(vectors::POW.len() >= 500);
    for &(x, y, expected) in vectors::POW {
        exact(
            strict_math::pow(f64::from_bits(x), f64::from_bits(y)),
            expected,
            "pow",
            x,
            Some(y),
        );
        checked += 1;
    }
    checked
}

#[test]
fn strict_math_seeded_reference_vectors_match_exact_bits() {
    assert_eq!(check_vectors(), 4381);
}
