//! Keyframe animations (vanilla's `AnimationDefinition`, `AnimationChannel`, `Keyframe`,
//! `KeyframeAnimation` and `AnimationState`) as plain data and pure functions. The game samples its
//! own mobs' animations through [`AnimationChannel::sample`], and a mod plays the files it ships
//! (`RenderAssets::animation`) on its own `ModelPart` with the same code, natively or in wasm.
use crate::{Identifier, ModelPart};

#[cfg(test)]
#[path = "keyframe_animation_tests.rs"]
mod tests;

/// What a channel moves (`AnimationChannel.Targets`): its sampled value is added to the part's
/// position, rotation or scale (`ModelPart.offsetPos`, `offsetRotation`, `offsetScale`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the three built-in targets and the registered ones
// no wire: crosses only inside `AnimationHolder`, whose wire form checks each index
pub enum AnimationTarget {
    /// The part's position, in texels (`minecraft:position`).
    Position,
    /// The part's rotation, in radians (`minecraft:rotation`).
    Rotation,
    /// The part's scale, as an offset from 1 (`minecraft:scale`).
    Scale,
    /// A target a mod registered (`register_json_animation_types`): its index in the
    /// animation's [`AnimationHolder::types`].
    Registered(u16),
}

/// How a channel reaches a keyframe from the one before (`AnimationChannel.Interpolations`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: the two built-in interpolations and the registered ones
// no wire: crosses only inside `AnimationHolder`, whose wire form checks each index
pub enum Interpolation {
    /// A straight line from the previous keyframe (`minecraft:linear`).
    Linear,
    /// A Catmull-Rom spline through the neighbouring keyframes (`minecraft:catmullrom`).
    CatmullRom,
    /// An interpolation a mod registered (`register_json_animation_types`): its index in the
    /// animation's [`AnimationHolder::types`].
    Registered(u16),
}

/// A target or interpolation id a mod registers for animation files to name
/// (`RegisterJsonAnimationTypesEvent.registerTarget` and `registerInterpolation`): pushed to
/// `register_json_animation_types`. What it does is code in the mod that plays the animation
/// ([`AnimationTypes`]).
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: NeoForge's event registers these two kinds
#[derive(horizon_mod_api_macros::Wire)]
pub enum JsonAnimationType {
    /// A channel target (`AnimationTarget`), `<namespace>:<path>` outside vanilla's namespace.
    Target(Identifier),
    /// A keyframe interpolation (`AnimationChannel.Interpolation`), `<namespace>:<path>` outside
    /// vanilla's namespace.
    Interpolation(Identifier),
}

/// One keyframe (`Keyframe`): at `time` seconds, the value arriving (`pre`) and the value leaving
/// (`post`), and how the channel moves into it.
#[derive(Clone, Copy, Debug, PartialEq)]
// exhaustive: plain data a parser builds and the sampler reads
// no wire: crosses only inside `AnimationHolder`, whose wire form checks each index
pub struct Keyframe {
    /// When, in seconds from the animation's start.
    pub time: f32,
    /// The value the channel arrives at.
    pub pre: [f32; 3],
    /// The value it leaves from.
    pub post: [f32; 3],
    /// How the channel moves into this keyframe from the one before.
    pub interpolation: Interpolation,
}

/// One bone's keyframes for one target (`AnimationChannel`), in time order.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds and the sampler reads
// no wire: crosses only inside `AnimationHolder`, whose wire form checks each index
pub struct AnimationChannel {
    /// What it moves.
    pub target: AnimationTarget,
    /// Its keyframes, in time order.
    pub keyframes: Vec<Keyframe>,
}

impl AnimationChannel {
    /// `KeyframeAnimation.Entry.apply`'s interpolation: the channel's value at `seconds`, times
    /// `scale`. Before the first keyframe it holds the first one, after the last the last. `None`
    /// at a segment into a registered interpolation, whose answer is a mod's code: such a channel
    /// plays through [`TypedKeyframeAnimation`].
    pub fn sample(&self, seconds: f32, scale: f32) -> Option<[f32; 3]> {
        let mut registered = false;
        let value = self.sample_by(seconds, scale, |_, _, _, _, _, _| {
            registered = true;
            [0.0; 3]
        });
        (!registered).then_some(value)
    }
    /// [`Self::sample`], with `registered` answering a segment into a registered interpolation:
    /// its index, the keyframes, the segment's ends, how far along it is and the scale.
    fn sample_by(
        &self,
        seconds: f32,
        scale: f32,
        mut registered: impl FnMut(u16, &[Keyframe], usize, usize, f32, f32) -> [f32; 3],
    ) -> [f32; 3] {
        let k = &self.keyframes;
        if k.is_empty() {
            return [0.0; 3];
        }
        // Mth.binarySearch(0, len, i -> seconds <= k[i].time) - 1
        let first = k.iter().position(|f| seconds <= f.time).unwrap_or(k.len());
        let prev = first.saturating_sub(1);
        let next = (prev + 1).min(k.len() - 1);
        let delta = seconds - k[prev].time;
        let alpha = if next != prev {
            (delta / (k[next].time - k[prev].time)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        match k[next].interpolation {
            Interpolation::CatmullRom => {
                let p0 = k[prev.saturating_sub(1)].post;
                let p1 = k[prev].post;
                let p2 = k[next].post;
                let p3 = k[(next + 1).min(k.len() - 1)].post;
                std::array::from_fn(|i| catmullrom(alpha, p0[i], p1[i], p2[i], p3[i]) * scale)
            }
            Interpolation::Linear => {
                let a = k[prev].post;
                let b = k[next].pre;
                std::array::from_fn(|i| (a[i] + (b[i] - a[i]) * alpha) * scale)
            }
            Interpolation::Registered(index) => registered(index, k, prev, next, alpha, scale),
        }
    }
}

/// `Mth.catmullrom`.
fn catmullrom(t: f32, p0: f32, p1: f32, p2: f32, p3: f32) -> f32 {
    0.5 * (2.0 * p1
        + (p2 - p0) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t * t
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t * t * t)
}

/// A target or interpolation a mod registered (`register_json_animation_types`) that an
/// animation's file names, with the mod that registered it.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: plain data the loader builds
#[derive(horizon_mod_api_macros::Wire)]
pub struct RegisteredAnimationType {
    /// Whether it is a target or an interpolation, and the id the file names (`mymod:glow`).
    pub kind: JsonAnimationType,
    /// The `[mod] id` of the mod that registered it.
    pub owner: String,
}

impl RegisteredAnimationType {
    /// The id the file names.
    pub fn id(&self) -> &Identifier {
        match &self.kind {
            JsonAnimationType::Target(id) | JsonAnimationType::Interpolation(id) => id,
        }
    }
}

/// An animation as its file gives it, bones by name (vanilla's `AnimationDefinition`; NeoForge's
/// `AnimationHolder`): what `RenderAssets::animation` answers. Bake it against a model with
/// [`AnimationHolder::bake`], or [`AnimationHolder::bake_with`] when it names registered types,
/// before playing it.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: plain data a parser builds
#[derive(horizon_mod_api_macros::Wire)]
#[wire(via = AnimationHolderWire)]
pub struct AnimationHolder {
    /// The animation's id (`mymod:golem/attack`), which bake errors name.
    pub id: Identifier,
    /// The animation's length in seconds.
    pub length: f32,
    /// Whether it starts over at the end (`loop`).
    pub looping: bool,
    /// Each bone's channels, by the part's name, in file order.
    pub bones: Vec<(String, Vec<AnimationChannel>)>,
    /// The registered targets and interpolations the file names, which `Registered` indexes
    /// point into. A registered target's values are as the file wrote them: its
    /// [`AnimationTypes::convert_target`] converts them at bake.
    pub types: Vec<RegisteredAnimationType>,
}

/// [`AnimationHolder`] as it crosses the wire, its channels in their own wire forms: decoding
/// refuses a holder whose `Registered` index doesn't name a type of its kind in its own `types`,
/// so an index never crosses without its list.
#[derive(horizon_mod_api_macros::Wire)]
struct AnimationHolderWire {
    id: Identifier,
    length: f32,
    looping: bool,
    bones: Vec<(String, Vec<ChannelWire>)>,
    types: Vec<RegisteredAnimationType>,
}

/// An [`AnimationChannel`] inside [`AnimationHolderWire`].
#[derive(horizon_mod_api_macros::Wire)]
struct ChannelWire {
    target: TargetWire,
    keyframes: Vec<KeyframeWire>,
}

/// A [`Keyframe`] inside [`AnimationHolderWire`].
#[derive(horizon_mod_api_macros::Wire)]
struct KeyframeWire {
    time: f32,
    pre: [f32; 3],
    post: [f32; 3],
    interpolation: InterpolationWire,
}

/// An [`AnimationTarget`] inside [`AnimationHolderWire`].
#[derive(horizon_mod_api_macros::Wire)]
enum TargetWire {
    Position,
    Rotation,
    Scale,
    Registered(u16),
}

/// An [`Interpolation`] inside [`AnimationHolderWire`].
#[derive(horizon_mod_api_macros::Wire)]
enum InterpolationWire {
    Linear,
    CatmullRom,
    Registered(u16),
}

impl From<&AnimationHolder> for AnimationHolderWire {
    fn from(holder: &AnimationHolder) -> Self {
        let channel = |channel: &AnimationChannel| ChannelWire {
            target: match channel.target {
                AnimationTarget::Position => TargetWire::Position,
                AnimationTarget::Rotation => TargetWire::Rotation,
                AnimationTarget::Scale => TargetWire::Scale,
                AnimationTarget::Registered(index) => TargetWire::Registered(index),
            },
            keyframes: channel
                .keyframes
                .iter()
                .map(|k| KeyframeWire {
                    time: k.time,
                    pre: k.pre,
                    post: k.post,
                    interpolation: match k.interpolation {
                        Interpolation::Linear => InterpolationWire::Linear,
                        Interpolation::CatmullRom => InterpolationWire::CatmullRom,
                        Interpolation::Registered(index) => InterpolationWire::Registered(index),
                    },
                })
                .collect(),
        };
        Self {
            id: holder.id.clone(),
            length: holder.length,
            looping: holder.looping,
            bones: holder
                .bones
                .iter()
                .map(|(bone, channels)| (bone.clone(), channels.iter().map(channel).collect()))
                .collect(),
            types: holder.types.clone(),
        }
    }
}

impl TryFrom<AnimationHolderWire> for AnimationHolder {
    type Error = AnimationError;
    fn try_from(wire: AnimationHolderWire) -> Result<Self, AnimationError> {
        let channel = |channel: ChannelWire| AnimationChannel {
            target: match channel.target {
                TargetWire::Position => AnimationTarget::Position,
                TargetWire::Rotation => AnimationTarget::Rotation,
                TargetWire::Scale => AnimationTarget::Scale,
                TargetWire::Registered(index) => AnimationTarget::Registered(index),
            },
            keyframes: channel
                .keyframes
                .into_iter()
                .map(|k| Keyframe {
                    time: k.time,
                    pre: k.pre,
                    post: k.post,
                    interpolation: match k.interpolation {
                        InterpolationWire::Linear => Interpolation::Linear,
                        InterpolationWire::CatmullRom => Interpolation::CatmullRom,
                        InterpolationWire::Registered(index) => Interpolation::Registered(index),
                    },
                })
                .collect(),
        };
        let holder = Self {
            id: wire.id,
            length: wire.length,
            looping: wire.looping,
            bones: wire
                .bones
                .into_iter()
                .map(|(bone, channels)| (bone, channels.into_iter().map(channel).collect()))
                .collect(),
            types: wire.types,
        };
        holder.check_indexes()?;
        Ok(holder)
    }
}

/// What a mod's registered targets and interpolations do (the `AnimationTarget` and
/// `AnimationChannel.Interpolation` a `RegisterJsonAnimationTypesEvent` listener registers). The
/// mod that plays an animation gives them to [`AnimationHolder::bake_with`], and they run in that
/// mod, called directly while it plays: no hook is dispatched per sample. An id one mod
/// registered loads in any mod's file, but plays only where an implementation of it exists: the
/// registering mod's own, or one a mod that depends on it supplies.
pub trait AnimationTypes {
    /// Whether this implementation plays `kind`, a registered target or interpolation: a target
    /// and an interpolation of one id are separate types.
    fn provides(&self, kind: &JsonAnimationType) -> bool;
    /// The target's conversion of a keyframe's file values, applied once at bake as the built-in
    /// targets apply `posVec`, `degreeVec` and `scaleVec`; the values unchanged by default.
    fn convert_target(&self, target: &Identifier, value: [f32; 3]) -> [f32; 3] {
        let _ = target;
        value
    }
    /// The target's `apply`: puts the sampled `value` on `part`.
    fn apply_target(&self, target: &Identifier, part: &mut ModelPart, value: [f32; 3]);
    /// The interpolation's answer for the segment from `keyframes[prev]` to `keyframes[next]`,
    /// `alpha` of the way along (0 to 1, or NaN where vanilla's is: two keyframes at one time, or a
    /// NaN time), times `scale`. `prev` and `next` are valid indexes.
    fn interpolate(
        &self,
        interpolation: &Identifier,
        keyframes: &[Keyframe],
        prev: usize,
        next: usize,
        alpha: f32,
        scale: f32,
    ) -> [f32; 3];
}

/// Why an animation can't be baked.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: `bake` returns it to the mod that bakes; it names an index without its list.
pub enum AnimationError {
    /// It names registered targets or interpolations: bake it with
    /// [`AnimationHolder::bake_with`] and the code that plays them.
    NeedsTypes {
        /// The animation's id.
        animation: Identifier,
        /// The registered types it names.
        types: Vec<RegisteredAnimationType>,
    },
    /// The supplied [`AnimationTypes`] doesn't play `missing`, which its `owner` registered.
    Unplayable {
        /// The animation's id.
        animation: Identifier,
        /// The first of its types the code doesn't provide.
        missing: RegisteredAnimationType,
    },
    /// A channel or keyframe names `Registered(index)`, which [`AnimationHolder::types`] lacks,
    /// or holds as the other kind (a target where an interpolation is named, or the reverse).
    NoSuchType {
        /// The animation's id.
        animation: Identifier,
        /// The index out of range, or naming the other kind.
        index: u16,
    },
}

impl std::fmt::Display for AnimationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = |t: &RegisteredAnimationType| format!("{} (registered by {})", t.id(), t.owner);
        match self {
            Self::NeedsTypes { animation, types } => write!(
                f,
                "animation {animation} names registered types, which need bake_with: {}",
                types.iter().map(name).collect::<Vec<_>>().join(", ")
            ),
            Self::Unplayable { animation, missing } => write!(
                f,
                "animation {animation}: no implementation of {} was supplied",
                name(missing)
            ),
            Self::NoSuchType { animation, index } => write!(
                f,
                "animation {animation} names registered type {index}, not in its types as that kind"
            ),
        }
    }
}

impl std::error::Error for AnimationError {}

impl AnimationHolder {
    /// `AnimationDefinition.bake(root)`: each bone resolved once through the model's part lookup
    /// (`ModelPart.createPartLookup`). The name `root` always selects the actual root; other names
    /// select the first match, visiting each part's immediate children before their descendants.
    /// Bake once, as a renderer does in its constructor, and keep the result: a bone the model
    /// lacks is logged here and left out. An animation that names registered types is refused,
    /// naming them: bake it with [`Self::bake_with`].
    pub fn bake(&self, root: &ModelPart) -> Result<KeyframeAnimation, AnimationError> {
        self.check_indexes()?;
        if !self.types.is_empty() {
            return Err(AnimationError::NeedsTypes {
                animation: self.id.clone(),
                types: self.types.clone(),
            });
        }
        Ok(self.baked(root, |_, value| value))
    }
    /// [`Self::bake`] for an animation that names registered types, with `types`, the code that
    /// plays them, which the result keeps: each registered target's keyframe values go through
    /// its [`AnimationTypes::convert_target`]. A type `types` doesn't provide is refused here,
    /// once, naming the animation and the mod that registered the type.
    pub fn bake_with<T: AnimationTypes>(
        &self,
        root: &ModelPart,
        types: T,
    ) -> Result<TypedKeyframeAnimation<T>, AnimationError> {
        self.check_indexes()?;
        if let Some(missing) = self.types.iter().find(|t| !types.provides(&t.kind)) {
            return Err(AnimationError::Unplayable {
                animation: self.id.clone(),
                missing: missing.clone(),
            });
        }
        let animation = self.baked(root, |target, value| match target {
            // In range: `check_indexes` refused any other.
            AnimationTarget::Registered(index) => {
                types.convert_target(self.types[usize::from(index)].id(), value)
            }
            _ => value,
        });
        Ok(TypedKeyframeAnimation {
            animation,
            types: self.types.clone(),
            code: types,
        })
    }
    /// Refuses a `Registered` index [`Self::types`] lacks or holds as the other kind, so playing
    /// never meets one.
    fn check_indexes(&self) -> Result<(), AnimationError> {
        let is_target = |kind: &JsonAnimationType| matches!(kind, JsonAnimationType::Target(_));
        let is_interpolation =
            |kind: &JsonAnimationType| matches!(kind, JsonAnimationType::Interpolation(_));
        for channel in self.bones.iter().flat_map(|(_, channels)| channels) {
            if let AnimationTarget::Registered(index) = channel.target {
                self.check_index(index, is_target)?;
            }
            for keyframe in &channel.keyframes {
                if let Interpolation::Registered(index) = keyframe.interpolation {
                    self.check_index(index, is_interpolation)?;
                }
            }
        }
        Ok(())
    }
    /// Refuses `index` unless [`Self::types`] holds a type of the kind `is_kind` accepts there.
    fn check_index(
        &self,
        index: u16,
        is_kind: impl Fn(&JsonAnimationType) -> bool,
    ) -> Result<(), AnimationError> {
        match self.types.get(usize::from(index)) {
            Some(registered) if is_kind(&registered.kind) => Ok(()),
            _ => Err(AnimationError::NoSuchType {
                animation: self.id.clone(),
                index,
            }),
        }
    }
    fn baked(
        &self,
        root: &ModelPart,
        convert: impl Fn(AnimationTarget, [f32; 3]) -> [f32; 3],
    ) -> KeyframeAnimation {
        let mut entries = Vec::new();
        for (bone, channels) in &self.bones {
            let Some(path) = root.descendant_path(bone) else {
                log::warn!("keyframe animation: the model has no part {bone:?}; skipped");
                continue;
            };
            for channel in channels {
                let mut channel = channel.clone();
                for keyframe in &mut channel.keyframes {
                    keyframe.pre = convert(channel.target, keyframe.pre);
                    keyframe.post = convert(channel.target, keyframe.post);
                }
                entries.push((path.clone(), channel));
            }
        }
        KeyframeAnimation {
            length: self.length,
            looping: self.looping,
            entries,
        }
    }
}

/// An animation baked against a model (`KeyframeAnimation`), of the built-in targets and
/// interpolations: each channel holds the path to its part, so playing it walks no names.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: built by `AnimationHolder::bake` only; its fields are private
// no wire: baked and played in the mod; a `Registered` index crosses only in its holder's types.
pub struct KeyframeAnimation {
    pub(crate) length: f32,
    pub(crate) looping: bool,
    pub(crate) entries: Vec<(Vec<u32>, AnimationChannel)>,
}

impl KeyframeAnimation {
    /// `KeyframeAnimation.apply(state, ageInTicks)`: while `state` is started, the pose
    /// `age_in_ticks` gives, added to `root`'s parts.
    pub fn apply(&self, root: &mut ModelPart, state: &AnimationState, age_in_ticks: f32) {
        self.apply_scaled(root, state, age_in_ticks, 1.0);
    }
    /// `KeyframeAnimation.apply(state, ageInTicks, speedFactor)`: as [`Self::apply`], played
    /// `speed_factor` times as fast.
    pub fn apply_scaled(
        &self,
        root: &mut ModelPart,
        state: &AnimationState,
        age_in_ticks: f32,
        speed_factor: f32,
    ) {
        if let Some(millis) = state.millis_at(age_in_ticks, speed_factor) {
            self.apply_at(root, millis, 1.0, None);
        }
    }
    /// `KeyframeAnimation.applyWalk(walkAnimationPos, walkAnimationSpeed, speedFactor,
    /// scaleFactor)`: the animation timed by a walk cycle, its strength by the walk speed.
    pub fn apply_walk(
        &self,
        root: &mut ModelPart,
        walk_pos: f32,
        walk_speed: f32,
        speed_factor: f32,
        scale_factor: f32,
    ) {
        let (millis, scale) = walk_timing(walk_pos, walk_speed, speed_factor, scale_factor);
        self.apply_at(root, millis, scale, None);
    }
    /// `KeyframeAnimation.applyStatic()`: the first frame only.
    pub fn apply_static(&self, root: &mut ModelPart) {
        self.apply_at(root, 0, 1.0, None);
    }
    /// `KeyframeAnimation.apply(millis, scale)`, with the registered types' code and ids for an
    /// animation [`AnimationHolder::bake_with`] baked.
    fn apply_at(
        &self,
        root: &mut ModelPart,
        millis: i64,
        scale: f32,
        registered: Option<(&dyn AnimationTypes, &[RegisteredAnimationType])>,
    ) {
        // getElapsedSeconds: a looping animation wraps by its length.
        let mut seconds = millis as f32 / 1000.0;
        if self.looping {
            seconds %= self.length;
        }
        for (path, channel) in &self.entries {
            let Some(part) = root.descendant_mut(path) else {
                continue;
            };
            // A `Registered` index is in range and has its code: `bake` refuses one, and
            // `bake_with` checks each against the types it was given.
            let value = channel.sample_by(seconds, scale, |index, k, prev, next, alpha, scale| {
                match registered {
                    Some((types, ids)) => {
                        types.interpolate(ids[usize::from(index)].id(), k, prev, next, alpha, scale)
                    }
                    // Without the code there is no registered segment: only `bake` builds a
                    // `KeyframeAnimation`, and it refuses one.
                    None => k[prev].post.map(|v| v * scale),
                }
            });
            match channel.target {
                AnimationTarget::Position => part.offset_pos(value),
                AnimationTarget::Rotation => part.offset_rotation(value),
                AnimationTarget::Scale => part.offset_scale(value),
                // Without the code there is no registered channel: only `bake` builds a
                // `KeyframeAnimation`, and it refuses one.
                AnimationTarget::Registered(index) => {
                    if let Some((types, ids)) = registered {
                        types.apply_target(ids[usize::from(index)].id(), part, value);
                    }
                }
            }
        }
    }
}

/// An animation that names registered targets or interpolations, baked with the code that plays
/// them ([`AnimationHolder::bake_with`]), which it keeps: it plays as [`KeyframeAnimation`] does,
/// and every registered channel runs that code.
#[derive(Clone, Debug)]
// exhaustive: built by `AnimationHolder::bake_with` only; its fields are private
// no wire: holds the playing mod's code, which stays in that mod.
pub struct TypedKeyframeAnimation<T> {
    animation: KeyframeAnimation,
    /// The registered types its `Registered` indexes name.
    types: Vec<RegisteredAnimationType>,
    /// The code that plays them, which provides every one.
    code: T,
}

impl<T: AnimationTypes> TypedKeyframeAnimation<T> {
    /// [`KeyframeAnimation::apply`].
    pub fn apply(&self, root: &mut ModelPart, state: &AnimationState, age_in_ticks: f32) {
        self.apply_scaled(root, state, age_in_ticks, 1.0);
    }
    /// [`KeyframeAnimation::apply_scaled`].
    pub fn apply_scaled(
        &self,
        root: &mut ModelPart,
        state: &AnimationState,
        age_in_ticks: f32,
        speed_factor: f32,
    ) {
        if let Some(millis) = state.millis_at(age_in_ticks, speed_factor) {
            self.animation
                .apply_at(root, millis, 1.0, Some(self.registered()));
        }
    }
    /// [`KeyframeAnimation::apply_walk`].
    pub fn apply_walk(
        &self,
        root: &mut ModelPart,
        walk_pos: f32,
        walk_speed: f32,
        speed_factor: f32,
        scale_factor: f32,
    ) {
        let (millis, scale) = walk_timing(walk_pos, walk_speed, speed_factor, scale_factor);
        self.animation
            .apply_at(root, millis, scale, Some(self.registered()));
    }
    /// [`KeyframeAnimation::apply_static`].
    pub fn apply_static(&self, root: &mut ModelPart) {
        self.animation
            .apply_at(root, 0, 1.0, Some(self.registered()));
    }
    fn registered(&self) -> (&dyn AnimationTypes, &[RegisteredAnimationType]) {
        (&self.code, &self.types)
    }
}

/// `applyWalk`'s timing: `walkAnimationPos * 50 * speedFactor` ms, at
/// `min(walkAnimationSpeed * scaleFactor, 1)` strength.
fn walk_timing(walk_pos: f32, walk_speed: f32, speed_factor: f32, scale_factor: f32) -> (i64, f32) {
    let millis = (walk_pos * 50.0 * speed_factor) as i64;
    let strength = walk_speed * scale_factor;
    // Java Math.min propagates NaN; Rust's f32::min would turn it into full strength.
    let scale = if strength.is_nan() {
        strength
    } else {
        strength.min(1.0)
    };
    (millis, scale)
}

/// When an animation started (vanilla's `AnimationState`): a start tick, or stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: its field is private; it changes through vanilla's transitions
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct AnimationState {
    pub(crate) start_tick: i32,
}

impl Default for AnimationState {
    fn default() -> Self {
        Self {
            start_tick: Self::STOPPED,
        }
    }
}

impl AnimationState {
    /// `AnimationState.STOPPED`.
    const STOPPED: i32 = i32::MIN;
    /// `new AnimationState()`: stopped.
    pub fn new() -> Self {
        Self::default()
    }
    /// `AnimationState.start(tickCount)`.
    pub fn start(&mut self, tick_count: i32) {
        self.start_tick = tick_count;
    }
    /// `AnimationState.startIfStopped(tickCount)`.
    pub fn start_if_stopped(&mut self, tick_count: i32) {
        if !self.is_started() {
            self.start(tick_count);
        }
    }
    /// `AnimationState.animateWhen(condition, tickCount)`: started while `condition` holds,
    /// stopped otherwise.
    pub fn animate_when(&mut self, condition: bool, tick_count: i32) {
        if condition {
            self.start_if_stopped(tick_count);
        } else {
            self.stop();
        }
    }
    /// `AnimationState.stop()`.
    pub fn stop(&mut self) {
        self.start_tick = Self::STOPPED;
    }
    /// `AnimationState.fastForward(ticks, speed)`: a started animation is `ticks * speed` further.
    pub fn fast_forward(&mut self, ticks: i32, speed: f32) {
        if self.is_started() {
            // Java's int subtraction wraps; so does this, and it never panics.
            self.start_tick = self.start_tick.wrapping_sub((ticks as f32 * speed) as i32);
        }
    }
    /// `AnimationState.getTimeInMillis(ageInTicks)`: how long it has run, 50 ms a tick.
    pub fn time_in_millis(&self, age_in_ticks: f32) -> i64 {
        ((age_in_ticks - self.start_tick as f32) * 50.0) as i64
    }
    /// `AnimationState.isStarted()`.
    pub fn is_started(&self) -> bool {
        self.start_tick != Self::STOPPED
    }
    /// `KeyframeAnimation.apply(state, ageInTicks, speedFactor)`'s time: while started,
    /// `(long)(getTimeInMillis(ageInTicks) * speedFactor)`.
    fn millis_at(&self, age_in_ticks: f32, speed_factor: f32) -> Option<i64> {
        self.is_started()
            .then(|| (self.time_in_millis(age_in_ticks) as f32 * speed_factor) as i64)
    }
}
