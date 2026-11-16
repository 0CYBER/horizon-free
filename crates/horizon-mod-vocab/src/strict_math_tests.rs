use super::*;

#[cfg_attr(test, test)]
pub(super) fn strict_math_literal_bits_on_every_target() {
    // First three sets and hypot are the existing jmath check1/check2 oracle values.
    for (f, cases) in [
        (
            log as fn(f64) -> f64,
            [
                (0x40569150fe105e6f, 0x401202e102f27b78),
                (0x3ff5dec70f518390, 0x3fd4009cda1f8856),
            ],
        ),
        (
            log10,
            [
                (0x3ff5dec70f518390, 0x3fc15fb52aa5f051),
                (0x3feb3d17325d493e, 0xbfb1e933cae02f5f),
            ],
        ),
        (
            cbrt,
            [
                (0x401bb21f8568bd0c, 0x3ffe7ee30c7a56f9),
                (0x3fc59f3a45c8fb80, 0x3fe1b0791e0f8a74),
            ],
        ),
    ] {
        for (x, bits) in cases {
            assert_eq!(f(f64::from_bits(x)).to_bits(), bits);
        }
    }
    for (x, y, bits) in [
        (0xc0569150fe105e6f, 0xc0056b970bf3b41a, 0x405693db72f461a6),
        (0x3fdc5999df9270f4, 0xc006c3ad990f72c0, 0x400709de26a1e40a),
    ] {
        assert_eq!(hypot(f64::from_bits(x), f64::from_bits(y)).to_bits(), bits);
    }
    // Obtained once from the existing jmath implementations; cross-checked
    // with Java 25 StrictMath. These literal expectations also run on wasm.
    for (f, bits) in [
        (
            atan as fn(f64) -> f64,
            [0x3fddac670561bb4f, 0xbfe4978fa3269ee1],
        ),
        (asin, [0x3fe0c152382d7366, 0xbfeb235315c680dc]),
        (acos, [0x3ff0c152382d7366, 0x400359d26f93b6c3]),
    ] {
        for (x, expected) in [0.5, -0.75].into_iter().zip(bits) {
            assert_eq!(f(x).to_bits(), expected);
        }
    }
    assert_eq!(atan2(1.0, 1.0).to_bits(), 0x3fe921fb54442d18);
    assert_eq!(atan2(1.0, -1.0).to_bits(), 0x4002d97c7f3321d2);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn java_math_matches_native_engine_paths() {
    use crate::java_math as api;
    use horizon_core::jmath as engine;
    for (a, b) in [
        (api::sin as fn(f64) -> f64, engine::sin as fn(f64) -> f64),
        (api::cos, engine::cos),
        (api::tan, engine::tan),
        (api::exp, engine::exp),
        (api::log, engine::log),
        (api::log10, engine::log10),
        (api::cbrt, engine::cbrt),
        (api::atan, engine::atan),
        (api::asin, engine::asin),
        (api::acos, engine::acos),
    ] {
        for x in [0.5, -0.75] {
            assert_eq!(a(x).to_bits(), b(x).to_bits());
        }
    }
    for (a, b) in [
        (
            api::pow as fn(f64, f64) -> f64,
            engine::pow as fn(f64, f64) -> f64,
        ),
        (api::atan2, engine::atan2),
        (api::hypot, engine::hypot),
    ] {
        assert_eq!(a(0.5, 2.0).to_bits(), b(0.5, 2.0).to_bits());
    }
    assert_eq!(api::round(-2.5), engine::round(-2.5));
    assert_eq!(api::round_f32(-2.5), engine::round_f32(-2.5));
}
