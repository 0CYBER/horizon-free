//! The `register_json_animation_types` hooks: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The animation targets and interpolations a mod adds for keyframe animation files to name
    /// (NeoForge's `RegisterJsonAnimationTypesEvent`): push each id, `<namespace>:<path>`, as a
    /// `JsonAnimationType::Target` or `JsonAnimationType::Interpolation`. Asked once at start on
    /// the client, before resources load. An id in vanilla's namespace, not of that form, or
    /// registered already as the same kind is logged and left out.
    #[notes = r#"## Animation types

A keyframe animation file (`assets/<namespace>/neoforge/animations/entity/<path>.json`) names a target per
entry and an interpolation per keyframe: vanilla's `minecraft:position`, `minecraft:rotation` and
`minecraft:scale`, and `minecraft:linear` and `minecraft:catmullrom`. A mod adds its own (NeoForge's
`RegisterJsonAnimationTypesEvent`) in two steps. `Hooks::register_json_animation_types` pushes the ids once
at start, so files may name them: an id no mod registered still fails its file, naming it. The code that
plays them is an `AnimationTypes` the playing mod gives `AnimationHolder::bake_with`, which the baked
animation keeps and runs in that mod, with no hook per sample.

```rust,ignore
fn register_json_animation_types(
    &self,
    _context: &RegistrationContext,
    event: &mut RegisterJsonAnimationTypesEvent<'_>,
) {
    event.types.push(JsonAnimationType::Target(Identifier::parse("example:glow").unwrap()));
    event.types.push(JsonAnimationType::Interpolation(Identifier::parse("example:step").unwrap()));
}

struct Types;

impl AnimationTypes for Types {
    fn provides(&self, kind: &JsonAnimationType) -> bool {
        match kind {
            JsonAnimationType::Target(id) => id.to_string() == "example:glow",
            JsonAnimationType::Interpolation(id) => id.to_string() == "example:step",
        }
    }
    fn apply_target(&self, _target: &Identifier, part: &mut ModelPart, value: [f32; 3]) {
        part.offset_scale(value);
    }
    fn interpolate(
        &self,
        _: &Identifier,
        keyframes: &[Keyframe],
        prev: usize,
        _: usize,
        _: f32,
        scale: f32,
    ) -> [f32; 3] {
        keyframes[prev].post.map(|v| v * scale)
    }
}

// In the renderer's constructor, once:
let baked = holder.bake_with(&root, Types)?;
// Each frame:
baked.apply(&mut root, &state, age_in_ticks);
```

A registered target's values reach `AnimationTypes::convert_target` once at bake, as the file wrote them
(the built-in targets convert theirs with vanilla's `posVec`, `degreeVec` and `scaleVec`); it returns them
unchanged by default. `AnimationHolder::bake` refuses an animation that names registered types, naming
them: bake it with `bake_with` instead, so no registered channel is skipped in silence.

An id one mod registered loads in any mod's file, but plays only where an `AnimationTypes` provides it:
the registering mod's own, or one a mod depending on it supplies. `bake_with` refuses an
`AnimationTypes` that lacks one of the animation's types, once, and the error names the animation, the
type and the mod that registered it."#]
    #[hook(
        side = "client",
        contract = "collect",
        out = "types",
        context = "registration",
        answered_by = "mod",
        neoforge = "RegisterJsonAnimationTypesEvent"
    )]
    fn register_json_animation_types();
}
