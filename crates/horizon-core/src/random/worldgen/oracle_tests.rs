//! Values read from the pinned 26.3 client jar itself (arismoko/horizon#815): a small Java
//! program called `LegacyRandomSource`, `WorldgenRandom.setDecorationSeed` and
//! `setFeatureSeed` against `client.jar` and printed the draws below. They pin what a
//! mod-visible draw does to the shared stream, and the decoration seeding of a feature.
use super::*;

fn bound(random: &mut impl RandomSource, bound: i32) -> i32 {
    random.next_int_bound(bound).unwrap()
}

#[test]
fn next_int_of_one_still_consumes_a_draw() {
    // Java: new LegacyRandomSource(0): nextInt(1)=0, then nextInt(16)=13,3, then
    // nextLong=-7261648964369397258.
    for mut random in [
        WorldgenRandom::new(LegacyRandom::new(0)),
        WorldgenRandom::plain(LegacyRandom::new(0)),
    ] {
        assert_eq!(bound(&mut random, 1), 0);
        assert_eq!(bound(&mut random, 16), 13);
        assert_eq!(bound(&mut random, 16), 3);
        assert_eq!(random.next_long(), -7261648964369397258);
    }
    let mut plain = LegacyRandom::new(0);
    assert_eq!(bound(&mut plain, 1), 0);
    assert_eq!(bound(&mut plain, 16), 13);
}

#[test]
fn decoration_and_feature_seeding_match_the_jar() {
    // Java: setDecorationSeed(0, 0, 0)=0, setFeatureSeed(0, 0, 0): nextInt(16)=11,13.
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    let seed = random.set_decoration_seed(0, 0, 0);
    assert_eq!(seed, 0);
    random.set_feature_seed(seed, 0, 0);
    assert_eq!((bound(&mut random, 16), bound(&mut random, 16)), (11, 13));
    // Java: setDecorationSeed(0, 16, -32)=-31007559772603472, setFeatureSeed(.., 3, 2):
    // nextInt(16)=10,4.
    let seed = random.set_decoration_seed(0, 16, -32);
    assert_eq!(seed, -31007559772603472);
    random.set_feature_seed(seed, 3, 2);
    assert_eq!((bound(&mut random, 16), bound(&mut random, 16)), (10, 4));
}

#[test]
fn the_xoroshiro_root_matches_the_jar() {
    // Java (with DataFixerUpper 9.0.19 on the class path): new XoroshiroRandomSource(0):
    // nextInt(1)=0, nextInt(16)=2,2, nextLong=2160572957309072155.
    let mut plain = XoroshiroRandom::new(0);
    assert_eq!(bound(&mut plain, 1), 0);
    assert_eq!(bound(&mut plain, 16), 2);
    assert_eq!(bound(&mut plain, 16), 2);
    assert_eq!(plain.next_long(), 2160572957309072155);
    // Java: new WorldgenRandom(new XoroshiroRandomSource(0)): nextInt(1)=0, nextInt(16)=12,4,
    // nextLong=2160572956399813066. The adapter reads the high bits of a long per draw.
    let mut wrapped = WorldgenRandom::new(XoroshiroRandom::new(0));
    assert_eq!(bound(&mut wrapped, 1), 0);
    assert_eq!(bound(&mut wrapped, 16), 12);
    assert_eq!(bound(&mut wrapped, 16), 4);
    assert_eq!(wrapped.next_long(), 2160572956399813066);
    // Java: setDecorationSeed(0, 16, -32)=-7422581477381670832, setFeatureSeed(.., 3, 2): 4,9.
    let mut random = WorldgenRandom::new(XoroshiroRandom::new(0));
    let seed = random.set_decoration_seed(0, 16, -32);
    assert_eq!(seed, -7422581477381670832);
    random.set_feature_seed(seed, 3, 2);
    assert_eq!((bound(&mut random, 16), bound(&mut random, 16)), (4, 9));
}
