use super::*;
use crate::{ModelPart, PartPose};

fn key(time: f32, value: [f32; 3], interpolation: Interpolation) -> Keyframe {
    Keyframe {
        time,
        pre: value,
        post: value,
        interpolation,
    }
}
fn channel(target: AnimationTarget, keyframes: Vec<Keyframe>) -> AnimationChannel {
    AnimationChannel { target, keyframes }
}
fn animation_id() -> Identifier {
    Identifier::parse("test:animation").unwrap()
}
fn part(children: Vec<(&str, ModelPart)>) -> ModelPart {
    ModelPart::new(
        Vec::new(),
        children
            .into_iter()
            .map(|(name, child)| (name.to_owned(), child))
            .collect(),
        PartPose::ZERO,
    )
}

#[test]
fn linear_keyframes_interpolate_post_to_pre() {
    let c = channel(
        AnimationTarget::Position,
        vec![
            key(0.0, [0.0; 3], Interpolation::Linear),
            key(1.0, [2.0, 4.0, 6.0], Interpolation::Linear),
        ],
    );
    assert_eq!(c.sample(0.5, 1.0), Some([1.0, 2.0, 3.0]));
    // Past the end it holds the last value, times the scale.
    assert_eq!(c.sample(3.0, 0.5), Some([1.0, 2.0, 3.0]));
    // Before the first, the first.
    assert_eq!(c.sample(-1.0, 1.0), Some([0.0; 3]));
    // A linear segment ends at the next keyframe's arriving value, not its leaving one.
    let jump = channel(
        AnimationTarget::Position,
        vec![
            key(0.0, [0.0; 3], Interpolation::Linear),
            Keyframe {
                time: 1.0,
                pre: [2.0; 3],
                post: [10.0; 3],
                interpolation: Interpolation::Linear,
            },
        ],
    );
    assert_eq!(jump.sample(0.5, 1.0), Some([1.0; 3]));
}

#[test]
fn catmullrom_passes_through_keyframes() {
    let c = channel(
        AnimationTarget::Rotation,
        vec![
            key(0.0, [0.0; 3], Interpolation::CatmullRom),
            key(1.0, [1.0; 3], Interpolation::CatmullRom),
            key(2.0, [0.0; 3], Interpolation::CatmullRom),
        ],
    );
    assert!((c.sample(1.0, 1.0).unwrap()[0] - 1.0).abs() < 1e-6);
    // Mth.catmullrom(0.5, 0, 0, 1, 0) = 0.5 * (1 + (0 + 4 - 0) * 0.25 + (0 - 0 - 3 + 0) * 0.125)
    assert_eq!(c.sample(0.5, 1.0).unwrap()[0], 0.5625);
}

/// A bone resolves to the first part of its name, each part's children before its
/// grandchildren; a bone the model lacks is left out, and the rest still plays.
#[test]
fn bake_finds_the_first_part_of_a_name_and_drops_a_missing_bone() {
    let root = part(vec![
        ("body", part(vec![("arm", part(vec![]))])),
        ("arm", part(vec![])),
    ]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        types: Vec::new(),
        bones: vec![
            (
                "arm".into(),
                vec![channel(
                    AnimationTarget::Position,
                    vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
                )],
            ),
            (
                "tail".into(),
                vec![channel(
                    AnimationTarget::Position,
                    vec![key(0.0, [9.0; 3], Interpolation::Linear)],
                )],
            ),
        ],
    };
    let baked = holder.bake(&root).unwrap();
    assert_eq!(baked.entries.len(), 1, "the missing tail is dropped");
    let mut posed = root.clone();
    baked.apply_static(&mut posed);
    // The root's own `arm` (index 1) is found before body's.
    let arm = posed.get_child("arm").unwrap();
    assert_eq!((arm.x, arm.y, arm.z), (1.0, 2.0, 3.0));
    let inner = posed.get_child("body").unwrap().get_child("arm").unwrap();
    assert_eq!((inner.x, inner.y, inner.z), (0.0, 0.0, 0.0));
}

/// `apply` only plays a started state, at `(age - start) * 50` ms; a looping animation wraps
/// by its length; `apply_walk` times by `pos * 50 * speed_factor` and scales by
/// `min(speed * scale_factor, 1)`; every apply adds to the pose.
#[test]
fn apply_times_by_the_state_and_wraps_a_loop() {
    let root = part(vec![("head", part(vec![]))]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 2.0,
        looping: true,
        types: Vec::new(),
        bones: vec![(
            "head".into(),
            vec![channel(
                AnimationTarget::Rotation,
                vec![
                    key(0.0, [0.0; 3], Interpolation::Linear),
                    key(2.0, [4.0, 0.0, 0.0], Interpolation::Linear),
                ],
            )],
        )],
    };
    let baked = holder.bake(&root).unwrap();
    let head_x = |root: &ModelPart| root.get_child("head").unwrap().x_rot;

    // Not looping, any time past the start would hold the last value, 4.0.
    let once = AnimationHolder {
        id: animation_id(),
        looping: false,
        ..holder.clone()
    }
    .bake(&root)
    .unwrap();
    let mut stopped = root.clone();
    once.apply(&mut stopped, &AnimationState::new(), 30.0);
    assert_eq!(head_x(&stopped), 0.0, "a stopped state plays nothing");

    let mut state = AnimationState::new();
    state.start(10);
    // 10 ticks after the start: 500 ms, a quarter of the way, 1.0.
    let mut posed = root.clone();
    baked.apply(&mut posed, &state, 20.0);
    assert_eq!(head_x(&posed), 1.0);
    // 50 ticks after: 2.5 s, wrapped to 0.5 s.
    let mut wrapped = root.clone();
    baked.apply(&mut wrapped, &state, 60.0);
    assert_eq!(head_x(&wrapped), 1.0);
    // Twice as fast: 1 s, halfway.
    let mut fast = root.clone();
    baked.apply_scaled(&mut fast, &state, 20.0, 2.0);
    assert_eq!(head_x(&fast), 2.0);

    // Walk position 20 at speed factor 1: 1000 ms, halfway; strength min(0.25 * 2, 1).
    let mut walk = root.clone();
    baked.apply_walk(&mut walk, 20.0, 0.25, 1.0, 2.0);
    assert_eq!(head_x(&walk), 1.0);
    // Applying again adds on top, as offsetRotation does.
    baked.apply_walk(&mut walk, 20.0, 0.25, 1.0, 2.0);
    assert_eq!(head_x(&walk), 2.0);
    // A strength over 1 is held at 1: min(1.5 * 2, 1) at halfway is 2.0.
    let mut strong = root.clone();
    baked.apply_walk(&mut strong, 20.0, 1.5, 1.0, 2.0);
    assert_eq!(head_x(&strong), 2.0);
}

/// Vanilla's transitions: `STOPPED` is `Integer.MIN_VALUE`, `animateWhen` starts once and stops,
/// `fastForward` moves the start back by `(int)(ticks * speed)` and wraps as Java's int does.
#[test]
fn animation_state_follows_vanillas_transitions() {
    let mut state = AnimationState::new();
    assert!(!state.is_started());
    state.animate_when(true, 5);
    state.animate_when(true, 9);
    assert_eq!(state.time_in_millis(7.0), 100, "started once, at 5");
    state.fast_forward(4, 0.5);
    assert_eq!(state.time_in_millis(7.0), 200, "now as if started at 3");
    state.animate_when(false, 9);
    assert!(!state.is_started());
    state.fast_forward(100, 1.0);
    assert!(!state.is_started(), "a stopped state doesn't move");
    state.start(i32::MIN + 1);
    state.fast_forward(2, 1.0);
    assert_eq!(
        state,
        AnimationState {
            start_tick: i32::MAX
        },
        "wraps, never panics"
    );
}

#[test]
fn oracle_root_bone_targets_actual_root_without_children() {
    let mut root = part(vec![]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        types: Vec::new(),
        bones: vec![(
            "root".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder.bake(&root).unwrap().apply_static(&mut root);
    assert_eq!((root.x, root.y, root.z), (1.0, 2.0, 3.0));
}

#[test]
fn oracle_root_bone_takes_precedence_over_child_named_root() {
    let mut root = part(vec![("root", part(vec![]))]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        types: Vec::new(),
        bones: vec![(
            "root".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder.bake(&root).unwrap().apply_static(&mut root);
    let child = root.get_child("root").unwrap();
    assert_eq!((child.x, child.y, child.z), (0.0, 0.0, 0.0));
    assert_eq!((root.x, root.y, root.z), (1.0, 2.0, 3.0));
}

#[test]
fn oracle_walk_nan_strength_propagates_like_java_math_min() {
    let mut root = part(vec![("head", part(vec![]))]);
    let holder = AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        types: Vec::new(),
        bones: vec![(
            "head".into(),
            vec![channel(
                AnimationTarget::Position,
                vec![key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear)],
            )],
        )],
    };
    holder
        .bake(&root)
        .unwrap()
        .apply_walk(&mut root, 0.0, f32::NAN, 1.0, 1.0);
    let head = root.get_child("head").unwrap();
    assert!(
        head.x.is_nan() && head.y.is_nan() && head.z.is_nan(),
        "Java Math.min(NaN, 1) propagates NaN, got {:?}",
        (head.x, head.y, head.z)
    );
}

fn glow(kind: fn(Identifier) -> JsonAnimationType, path: &str) -> RegisteredAnimationType {
    RegisteredAnimationType {
        kind: kind(Identifier::from_parts("glow", path).unwrap()),
        owner: "glow_mod".into(),
    }
}

/// A head with a registered target (`glow:pulse`) whose last keyframe arrives through a
/// registered interpolation (`glow:step`), and a built-in rotation beside it.
fn typed_holder() -> AnimationHolder {
    AnimationHolder {
        id: animation_id(),
        length: 1.0,
        looping: false,
        types: vec![
            glow(JsonAnimationType::Target, "pulse"),
            glow(JsonAnimationType::Interpolation, "step"),
        ],
        bones: vec![(
            "head".into(),
            vec![
                channel(
                    AnimationTarget::Registered(0),
                    vec![
                        key(0.0, [1.0, 2.0, 3.0], Interpolation::Linear),
                        key(1.0, [3.0, 4.0, 5.0], Interpolation::Registered(1)),
                    ],
                ),
                channel(
                    AnimationTarget::Rotation,
                    vec![
                        key(0.0, [0.0; 3], Interpolation::Linear),
                        key(1.0, [2.0, 0.0, 0.0], Interpolation::Linear),
                    ],
                ),
            ],
        )],
    }
}

/// The glow mod's code: its target triples the file's values at bake and offsets the part's
/// position; its interpolation answers the next keyframe's value times `alpha` and records how
/// it was called. It provides every glow type.
#[derive(Clone, Debug, Default)]
struct Glow {
    calls: std::rc::Rc<std::cell::RefCell<Vec<(String, usize, usize, f32, f32)>>>,
}

impl AnimationTypes for Glow {
    fn provides(&self, kind: &JsonAnimationType) -> bool {
        let (JsonAnimationType::Target(id) | JsonAnimationType::Interpolation(id)) = kind;
        id.namespace() == "glow"
    }
    fn convert_target(&self, _: &Identifier, value: [f32; 3]) -> [f32; 3] {
        value.map(|v| v * 3.0)
    }
    fn apply_target(&self, target: &Identifier, part: &mut ModelPart, value: [f32; 3]) {
        assert_eq!(target.to_string(), "glow:pulse");
        part.offset_pos(value);
    }
    fn interpolate(
        &self,
        interpolation: &Identifier,
        keyframes: &[Keyframe],
        prev: usize,
        next: usize,
        alpha: f32,
        scale: f32,
    ) -> [f32; 3] {
        self.calls
            .borrow_mut()
            .push((interpolation.to_string(), prev, next, alpha, scale));
        keyframes[next].pre.map(|v| v * alpha * scale)
    }
}

/// Provides nothing: a mod that plays another mod's animation without its code.
#[derive(Debug)]
struct NoTypes;

impl AnimationTypes for NoTypes {
    fn provides(&self, _: &JsonAnimationType) -> bool {
        false
    }
    fn apply_target(&self, _: &Identifier, _: &mut ModelPart, _: [f32; 3]) {
        unreachable!("never provided")
    }
    fn interpolate(
        &self,
        _: &Identifier,
        _: &[Keyframe],
        _: usize,
        _: usize,
        _: f32,
        _: f32,
    ) -> [f32; 3] {
        unreachable!("never provided")
    }
}

/// Half a second in: the registered segment runs the mod's interpolation on the converted
/// keyframes, (3, 4, 5) * 3 * 0.5 = (4.5, 6, 7.5), and its target puts that on the position; the
/// built-in rotation beside it is halfway to 2.
#[test]
fn registered_types_play_through_the_mods_code() {
    let root = part(vec![("head", part(vec![]))]);
    let glow = Glow::default();
    let baked = typed_holder().bake_with(&root, glow.clone()).unwrap();
    let mut state = AnimationState::new();
    state.start(0);
    let mut posed = root.clone();
    baked.apply(&mut posed, &state, 10.0);
    let head = posed.get_child("head").unwrap();
    assert_eq!((head.x, head.y, head.z), (4.5, 6.0, 7.5));
    assert_eq!(head.x_rot, 1.0);
    assert_eq!(
        *glow.calls.borrow(),
        [("glow:step".to_owned(), 0, 1, 0.5, 1.0)]
    );
}

/// An animation with registered types never plays without their code: `bake` refuses it, and so
/// does `bake_with` given code that lacks one, each naming the animation, the type and the mod
/// that registered it.
#[test]
fn baking_without_the_types_is_refused_naming_the_animation_and_mod() {
    let root = part(vec![("head", part(vec![]))]);
    let holder = typed_holder();
    let needs = holder.bake(&root).unwrap_err();
    assert_eq!(
        needs,
        AnimationError::NeedsTypes {
            animation: animation_id(),
            types: holder.types.clone()
        }
    );
    let unplayable = holder.bake_with(&root, NoTypes).unwrap_err();
    assert_eq!(
        unplayable,
        AnimationError::Unplayable {
            animation: animation_id(),
            missing: glow(JsonAnimationType::Target, "pulse")
        }
    );
    for error in [needs, unplayable] {
        let message = error.to_string();
        for named in ["test:animation", "glow:pulse", "glow_mod"] {
            assert!(message.contains(named), "{message:?} names {named}");
        }
    }
}

/// A holder is plain data a mod can build: an index one past its types is refused at bake rather
/// than panicking in play, and NaN times and values play without a panic.
#[test]
fn a_mods_holder_cannot_panic_the_player() {
    let root = part(vec![("head", part(vec![]))]);
    let mut holder = typed_holder();
    holder.bones[0].1[0].keyframes[1].interpolation = Interpolation::Registered(2);
    let out_of_range = AnimationError::NoSuchType {
        animation: animation_id(),
        index: 2,
    };
    assert_eq!(
        holder.bake_with(&root, Glow::default()).unwrap_err(),
        out_of_range
    );
    assert_eq!(holder.bake(&root).unwrap_err(), out_of_range);

    let mut holder = typed_holder();
    for keyframe in &mut holder.bones[0].1[0].keyframes {
        keyframe.time = f32::NAN;
        keyframe.pre = [f32::NAN; 3];
    }
    holder.length = f32::NAN;
    holder.looping = true;
    let baked = holder.bake_with(&root, Glow::default()).unwrap();
    let mut state = AnimationState::new();
    state.start(0);
    let mut posed = root.clone();
    baked.apply(&mut posed, &state, 10.0);
    baked.apply_walk(&mut posed, f32::NAN, f32::NAN, 1.0, 1.0);
    baked.apply_static(&mut posed);
    assert!(
        posed.get_child("head").unwrap().x.is_nan(),
        "the registered target got the NaN its keyframes give"
    );
}

/// A target and an interpolation of one id are separate types: a channel's target index naming an
/// interpolation is refused, and code that plays only targets can't bake an animation that also
/// names a registered interpolation.
#[test]
fn a_types_kind_counts() {
    let root = part(vec![("head", part(vec![]))]);
    let mut holder = typed_holder();
    holder.bones[0].1[0].target = AnimationTarget::Registered(1);
    assert_eq!(
        holder.bake_with(&root, Glow::default()).unwrap_err(),
        AnimationError::NoSuchType {
            animation: animation_id(),
            index: 1
        }
    );

    #[derive(Debug)]
    struct TargetsOnly(Glow);
    impl AnimationTypes for TargetsOnly {
        fn provides(&self, kind: &JsonAnimationType) -> bool {
            matches!(kind, JsonAnimationType::Target(_)) && self.0.provides(kind)
        }
        fn apply_target(&self, target: &Identifier, part: &mut ModelPart, value: [f32; 3]) {
            self.0.apply_target(target, part, value);
        }
        fn interpolate(
            &self,
            _: &Identifier,
            _: &[Keyframe],
            _: usize,
            _: usize,
            _: f32,
            _: f32,
        ) -> [f32; 3] {
            unreachable!("plays no interpolation")
        }
    }
    assert_eq!(
        typed_holder()
            .bake_with(&root, TargetsOnly(Glow::default()))
            .unwrap_err(),
        AnimationError::Unplayable {
            animation: animation_id(),
            missing: glow(JsonAnimationType::Interpolation, "step")
        }
    );
}

/// The built-in sampler has no mod code: at a segment into a registered interpolation it answers
/// `None`, and before it, where the segment is built in, the value.
#[test]
fn sampling_a_registered_segment_without_its_code_is_none() {
    let channel = &typed_holder().bones[0].1[0];
    assert_eq!(channel.sample(0.5, 1.0), None);
    assert_eq!(
        channel.sample(-1.0, 1.0),
        None,
        "held before the first keyframe, the segment into the second is still registered"
    );
    let built_in = &typed_holder().bones[0].1[1];
    assert_eq!(built_in.sample(0.5, 1.0), Some([1.0, 0.0, 0.0]));
}

/// A holder crosses the wire with its types, and decoding refuses one whose `Registered` index
/// its own list doesn't back, so no bare index crosses alone.
#[test]
fn a_holder_crosses_the_wire_only_with_the_types_its_indexes_name() {
    use crate::wire::{Reader, WireError, Writer};
    let decode = |holder: &AnimationHolder| {
        let mut out = Writer::new();
        out.put(holder);
        assert_eq!(out.error(), None);
        Reader::new(out.bytes()).take::<AnimationHolder>()
    };
    let holder = typed_holder();
    assert_eq!(decode(&holder), Ok(holder.clone()));
    // Every target and interpolation, and a keyframe arriving and leaving at different values,
    // come back as they went.
    let mut every = holder.clone();
    every.bones[0].1.extend(
        [AnimationTarget::Position, AnimationTarget::Scale].map(|target| AnimationChannel {
            target,
            keyframes: vec![
                Keyframe {
                    time: 0.25,
                    pre: [1.0, 2.0, 3.0],
                    post: [4.0, 5.0, 6.0],
                    interpolation: Interpolation::CatmullRom,
                },
                key(0.5, [7.0; 3], Interpolation::Linear),
            ],
        }),
    );
    assert_eq!(decode(&every), Ok(every.clone()));

    let mut past = holder.clone();
    past.bones[0].1[0].keyframes[1].interpolation = Interpolation::Registered(2);
    assert!(matches!(decode(&past), Err(WireError::Invalid(_))));
    let mut wrong_kind = holder;
    wrong_kind.bones[0].1[0].target = AnimationTarget::Registered(1);
    assert!(matches!(decode(&wrong_kind), Err(WireError::Invalid(_))));
}
