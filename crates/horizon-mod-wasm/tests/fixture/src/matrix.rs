//! Shared callbacks compiled unchanged into the native test and real wasm member.
pub fn sum_return(args: &mut mod_points::ListenerFixtureSumArgs) {
    assert_eq!(args.a, 3);
    assert_eq!(args.b, 7);
    assert_eq!(args.result, Some(10));
    args.result = Some(args.result.unwrap() + 2);
    args.b = 99; // The caller's shared source must still be 7.
}

pub fn mirrors(args: &mut mod_points::ListenerFixtureMirrorsArgs) {
    assert_eq!(args.text, mod_api::Text::literal("fixture: café 水"));
    assert_eq!(args.first.ordinal(), 0);
    assert_eq!(args.second.ordinal(), 1);
    assert_eq!(args.first.block(), args.second.block());
    args.cancel = true;
}
