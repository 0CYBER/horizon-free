//! Keyframe animations mods and packs ship (NeoForge's
//! `assets/<namespace>/neoforge/animations/entity/<path>.json`), read on every resource load into
//! the published render assets, where `RenderAssets::animation` finds them by id. The game's own
//! exported animations keep their reader (`mobs::keyframes`); both play through `mod_api`'s
//! sampling. The targets and interpolations mods register (NeoForge's
//! `RegisterJsonAnimationTypesEvent`) are kept here once at start, for files to name.
use mod_api::{
    AnimationChannel, AnimationHolder, AnimationTarget, Answered, Identifier, Interpolation,
    JsonAnimationType, Keyframe, RegisteredAnimationType,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::{Arc, RwLock},
};

/// The registered types of one kind, by id, each with the mod that registered it.
type Registered = RwLock<BTreeMap<String, RegisteredAnimationType>>;

static TARGETS: Registered = RwLock::new(BTreeMap::new());
static INTERPOLATIONS: Registered = RwLock::new(BTreeMap::new());

/// Keeps the animation targets and interpolations native mods registered, each with its mod,
/// before resources load. An id that is not `<namespace>:<path>` in a namespace other than
/// vanilla's, or one registered already as the same kind, is logged and left out: the first
/// registration of an id wins.
pub fn register(types: Vec<Answered<JsonAnimationType>>) {
    keep(types.into_iter().map(|entry| (entry.answer, entry.mod_id)));
}

/// [`register`]'s keeping, of each type with the `[mod] id` dispatch named for it. Returns the
/// types kept, in registration order.
fn keep<'a>(
    types: impl IntoIterator<Item = (JsonAnimationType, &'a str)>,
) -> Vec<RegisteredAnimationType> {
    let mut kept = Vec::new();
    for (kind, owner) in types {
        let (table, name, id) = match &kind {
            JsonAnimationType::Target(id) => (&TARGETS, "animation target", id.to_string()),
            JsonAnimationType::Interpolation(id) => {
                (&INTERPOLATIONS, "animation interpolation", id.to_string())
            }
        };
        if !horizon_assets::item_model::is_mod_id(&id) {
            log::warn!("{name} {id:?} is not <namespace>:<path> in a mod's namespace");
            continue;
        }
        match table.write().unwrap_or_else(|e| e.into_inner()).entry(id) {
            Entry::Occupied(first) => log::warn!(
                "{name} {} is registered twice: by {} and by {owner}",
                first.key(),
                first.get().owner
            ),
            Entry::Vacant(slot) => {
                let registered = RegisteredAnimationType {
                    kind,
                    owner: owner.to_owned(),
                };
                kept.push(slot.insert(registered).clone());
            }
        }
    }
    kept
}

/// The index in `types` of the type `table` registered as `id`, added the first time a file
/// names it; `None` when `table` has no `id`.
fn type_index(
    types: &mut Vec<RegisteredAnimationType>,
    table: &Registered,
    id: &str,
) -> Result<Option<u16>, String> {
    let table = table.read().unwrap_or_else(|e| e.into_inner());
    let Some(registered) = table.get(id) else {
        return Ok(None);
    };
    let index = match types.iter().position(|t| t == registered) {
        Some(index) => index,
        None => {
            types.push(registered.clone());
            types.len() - 1
        }
    };
    u16::try_from(index)
        .map(Some)
        .map_err(|_| "more registered types than an animation holds".to_owned())
}

/// The folder under each namespace the files are read from.
const FOLDER: &str = "neoforge/animations/entity";

/// The animations of one resource load, by id (`mymod:golem/attack` for
/// `assets/mymod/neoforge/animations/entity/golem/attack.json`).
#[derive(Clone, Debug, Default)]
pub struct Animations(BTreeMap<Identifier, Arc<AnimationHolder>>);

impl Animations {
    /// Every animation file of `source`. A file that doesn't parse is logged with its reason and
    /// left out; the others load.
    pub fn load(source: &crate::resource_source::ResourceSource) -> Self {
        let mut out = BTreeMap::new();
        for file in source.list_under("assets", &[FOLDER]) {
            let Some((namespace, path)) = file
                .strip_prefix("assets/")
                .and_then(|rest| rest.split_once(&format!("/{FOLDER}/")))
            else {
                continue;
            };
            let Some(path) = path.strip_suffix(".json") else {
                continue;
            };
            let Ok(id) = Identifier::from_parts(namespace, path) else {
                log::warn!("Horizon: animation {file}: not a valid id; skipped");
                continue;
            };
            match source
                .read_to_string(&file)
                .map_err(|e| e.to_string())
                .and_then(|text| parse(id.clone(), &text))
            {
                Ok(holder) => {
                    out.insert(id, Arc::new(holder));
                }
                Err(error) => log::warn!("Horizon: animation {id} not loaded: {error}"),
            }
        }
        Self(out)
    }
    /// The animation `id`, if a file gave it.
    pub fn get(&self, id: &Identifier) -> Option<&AnimationHolder> {
        self.0.get(id).map(Arc::as_ref)
    }
}

/// One animation file: `length` (seconds), optional `loop`, and `animations`, entries of a
/// `bone`, a `target` and its `keyframes` (`timestamp`, `target` values, `interpolation`). Values
/// go through vanilla's builder helpers (`KeyframeAnimations`): a position's y is negated
/// (`posVec`), a rotation is in degrees (`degreeVec`), a scale is a factor (`scaleVec` keeps its
/// offset from 1). A registered target's values are kept as written: its mod converts them at
/// bake. A target or interpolation neither built in nor registered fails the file, naming it.
pub fn parse(animation: Identifier, text: &str) -> Result<AnimationHolder, String> {
    let json: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let length = finite(&json["length"]).ok_or("no finite `length`")? as f32;
    if length <= 0.0 {
        return Err(format!("`length` {length} is not positive"));
    }
    let looping = match json.get("loop") {
        None => false,
        Some(value) => value.as_bool().ok_or("`loop` is not a boolean")?,
    };
    let entries = json["animations"]
        .as_array()
        .ok_or("no `animations` list")?;
    let mut bones: Vec<(String, Vec<AnimationChannel>)> = Vec::new();
    let mut types = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let bone = entry["bone"]
            .as_str()
            .ok_or_else(|| format!("animations[{index}]: no `bone`"))?;
        let target_id = id(&entry["target"]);
        let target = match target_id.as_deref() {
            Some("minecraft:position") => AnimationTarget::Position,
            Some("minecraft:rotation") => AnimationTarget::Rotation,
            Some("minecraft:scale") => AnimationTarget::Scale,
            other => match other
                .map(|id| type_index(&mut types, &TARGETS, id))
                .transpose()
                .map_err(|e| format!("animations[{index}]: {e}"))?
            {
                Some(Some(registered)) => AnimationTarget::Registered(registered),
                _ => {
                    return Err(format!(
                        "animations[{index}]: unknown target {}",
                        other.unwrap_or("(none)")
                    ));
                }
            },
        };
        let keyframes = entry["keyframes"]
            .as_array()
            .ok_or_else(|| format!("animations[{index}]: no `keyframes` list"))?
            .iter()
            .enumerate()
            .map(|(at, keyframe)| {
                keyframe_of(keyframe, target, &mut types)
                    .map_err(|e| format!("animations[{index}].keyframes[{at}]: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let channel = AnimationChannel { target, keyframes };
        match bones.iter_mut().find(|(name, _)| name == bone) {
            Some((_, channels)) => channels.push(channel),
            None => bones.push((bone.to_owned(), vec![channel])),
        }
    }
    Ok(AnimationHolder {
        id: animation,
        length,
        looping,
        bones,
        types,
    })
}

/// A number that stays finite as an `f32`: 1e40 doesn't, it would become infinity.
fn finite(value: &Value) -> Option<f64> {
    let value = value.as_f64()?;
    (value as f32).is_finite().then_some(value)
}

/// A namespaced id, `minecraft:` when it names none (`Identifier.parse`).
fn id(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    Some(if text.contains(':') {
        text.to_owned()
    } else {
        format!("minecraft:{text}")
    })
}

fn keyframe_of(
    keyframe: &Value,
    target: AnimationTarget,
    types: &mut Vec<RegisteredAnimationType>,
) -> Result<Keyframe, String> {
    let time = finite(&keyframe["timestamp"]).ok_or("no finite `timestamp`")? as f32;
    let values = keyframe["target"]
        .as_array()
        .filter(|values| values.len() == 3)
        .ok_or("`target` is not three numbers")?;
    let mut v = [0.0f64; 3];
    for (slot, value) in v.iter_mut().zip(values) {
        *slot = finite(value).ok_or("`target` is not three finite numbers")?;
    }
    let value = match target {
        // KeyframeAnimations.posVec
        AnimationTarget::Position => [v[0] as f32, -(v[1] as f32), v[2] as f32],
        // KeyframeAnimations.degreeVec
        AnimationTarget::Rotation => v.map(|d| d as f32 * 0.017_453_292),
        // KeyframeAnimations.scaleVec
        AnimationTarget::Scale => v.map(|d| (d - 1.0) as f32),
        AnimationTarget::Registered(_) => v.map(|d| d as f32),
    };
    let interpolation_id = id(&keyframe["interpolation"]);
    let interpolation = match interpolation_id.as_deref() {
        Some("minecraft:linear") => Interpolation::Linear,
        Some("minecraft:catmullrom") => Interpolation::CatmullRom,
        other => match other
            .map(|id| type_index(types, &INTERPOLATIONS, id))
            .transpose()?
        {
            Some(Some(registered)) => Interpolation::Registered(registered),
            _ => {
                return Err(format!(
                    "unknown interpolation {}",
                    other.unwrap_or("(none)")
                ));
            }
        },
    };
    Ok(Keyframe {
        time,
        pre: value,
        post: value,
        interpolation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource_source::{Resource, ResourceSource};
    use mod_api::{AnimationState, ModelPart, PartPose};
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    const SWING: &str = r#"{"length": 2.0, "loop": true, "animations": [
        {"bone": "arm", "target": "minecraft:rotation", "keyframes": [
            {"timestamp": 0.0, "target": [0, 0, 0], "interpolation": "minecraft:linear"},
            {"timestamp": 1.0, "target": [90, 0, -45], "interpolation": "linear"}]},
        {"bone": "arm", "target": "minecraft:position", "keyframes": [
            {"timestamp": 0.0, "target": [1, 2, 3], "interpolation": "minecraft:catmullrom"}]},
        {"bone": "body", "target": "minecraft:scale", "keyframes": [
            {"timestamp": 0.0, "target": [2, 1, 0.5], "interpolation": "minecraft:linear"}]}
    ]}"#;

    /// A file of the test animation `test:animation`.
    fn read(text: &str) -> Result<AnimationHolder, String> {
        parse(Identifier::parse("test:animation").unwrap(), text)
    }

    fn key(holder: &AnimationHolder, bone: usize, channel: usize, at: usize) -> Keyframe {
        holder.bones[bone].1[channel].keyframes[at]
    }

    /// Values go through vanilla's builder helpers: degrees to radians, y negated, scale as an
    /// offset from 1. 90 and -45 degrees times `degreeVec`'s factor round to the f32 quarter and
    /// eighth turns exactly (worked out apart from this code).
    #[test]
    fn a_file_parses_with_vanillas_units_and_groups_its_bones() {
        let holder = read(SWING).unwrap();
        assert_eq!((holder.length, holder.looping), (2.0, true));
        let bones: Vec<&str> = holder.bones.iter().map(|(b, _)| b.as_str()).collect();
        assert_eq!(
            bones,
            ["arm", "body"],
            "two entries of `arm` share its bone"
        );
        let swing = key(&holder, 0, 0, 1);
        assert_eq!(swing.post, [FRAC_PI_2, 0.0, -FRAC_PI_4]);
        assert_eq!(swing.pre, swing.post);
        assert_eq!(
            swing.interpolation,
            Interpolation::Linear,
            "an unnamespaced id is minecraft's"
        );
        let at = key(&holder, 0, 1, 0);
        assert_eq!(at.post, [1.0, -2.0, 3.0]);
        assert_eq!(at.interpolation, Interpolation::CatmullRom);
        assert_eq!(key(&holder, 1, 0, 0).post, [1.0, 0.0, -0.5]);
        assert!(
            !read(r#"{"length":1,"animations":[]}"#).unwrap().looping,
            "loop defaults to false"
        );
    }

    #[test]
    fn an_unknown_target_or_interpolation_fails_the_file_by_name() {
        let bad_target =
            r#"{"length":1,"animations":[{"bone":"a","target":"mymod:glow","keyframes":[]}]}"#;
        assert_eq!(
            read(bad_target).unwrap_err(),
            "animations[0]: unknown target mymod:glow"
        );
        let bad_interpolation = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1,1],"interpolation":"mymod:bounce"}]}]}"#;
        assert_eq!(
            read(bad_interpolation).unwrap_err(),
            "animations[0].keyframes[0]: unknown interpolation mymod:bounce"
        );
        assert!(read(r#"{"animations":[]}"#).is_err(), "no length");
        assert_eq!(
            read(r#"{"length":0,"loop":true,"animations":[]}"#).unwrap_err(),
            "`length` 0 is not positive"
        );
        assert_eq!(
            read(r#"{"length":1e40,"animations":[]}"#).unwrap_err(),
            "no finite `length`"
        );
        let huge = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1e40,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert_eq!(
            read(huge).unwrap_err(),
            "animations[0].keyframes[0]: `target` is not three finite numbers"
        );
        let late = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":1e40,"target":[1,1,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert_eq!(
            read(late).unwrap_err(),
            "animations[0].keyframes[0]: no finite `timestamp`"
        );
        let short = r#"{"length":1,"animations":[{"bone":"a","target":"minecraft:scale",
            "keyframes":[{"timestamp":0,"target":[1,1],"interpolation":"minecraft:linear"}]}]}"#;
        assert!(read(short).is_err(), "two values");
    }

    #[test]
    fn registration_keeps_each_kind_once_with_its_mod() {
        let target = |id: &str| JsonAnimationType::Target(Identifier::parse(id).unwrap());
        let interpolation =
            |id: &str| JsonAnimationType::Interpolation(Identifier::parse(id).unwrap());
        let kept = keep([
            (target("animreg:glow"), "one"),
            (target("animreg:glow"), "two"),
            (interpolation("animreg:glow"), "two"),
            (target("minecraft:glow"), "one"),
        ]);
        let kept: Vec<_> = kept
            .iter()
            .map(|t| (t.kind.clone(), t.owner.as_str()))
            .collect();
        assert_eq!(
            kept,
            [
                (target("animreg:glow"), "one"),
                (interpolation("animreg:glow"), "two")
            ],
            "a second target of an id is refused, an interpolation of it is its own kind"
        );
    }

    /// A registered target keeps the file's values as written, its keyframes may use a
    /// registered interpolation, and each type is listed once with the mod that registered it.
    #[test]
    fn a_file_names_registered_types_by_index() {
        keep([
            (
                JsonAnimationType::Target(Identifier::parse("animfile:pulse").unwrap()),
                "pulser",
            ),
            (
                JsonAnimationType::Interpolation(Identifier::parse("animfile:step").unwrap()),
                "stepper",
            ),
        ]);
        let holder = read(
            r#"{"length": 1, "animations": [
            {"bone": "a", "target": "animfile:pulse", "keyframes": [
                {"timestamp": 0, "target": [90, 2, 1], "interpolation": "animfile:step"},
                {"timestamp": 1, "target": [0, 0, 0], "interpolation": "linear"}]},
            {"bone": "a", "target": "minecraft:position", "keyframes": [
                {"timestamp": 0, "target": [0, 2, 0], "interpolation": "animfile:step"}]}
        ]}"#,
        )
        .unwrap();
        let owners: Vec<_> = holder
            .types
            .iter()
            .map(|t| (t.id().to_string(), t.owner.as_str()))
            .collect();
        assert_eq!(
            owners,
            [
                ("animfile:pulse".to_owned(), "pulser"),
                ("animfile:step".to_owned(), "stepper")
            ]
        );
        let channels = &holder.bones[0].1;
        assert_eq!(channels[0].target, AnimationTarget::Registered(0));
        let first = key(&holder, 0, 0, 0);
        assert_eq!(first.post, [90.0, 2.0, 1.0], "not converted to radians");
        assert_eq!(first.interpolation, Interpolation::Registered(1));
        let built_in = key(&holder, 0, 1, 0);
        assert_eq!(built_in.post, [0.0, -2.0, 0.0], "posVec still applies");
        assert_eq!(
            built_in.interpolation,
            Interpolation::Registered(1),
            "the second use shares the first's index"
        );
        let huge = r#"{"length":1,"animations":[{"bone":"a","target":"animfile:pulse",
            "keyframes":[{"timestamp":0,"target":[1e40,0,0],"interpolation":"linear"}]}]}"#;
        assert_eq!(
            read(huge).unwrap_err(),
            "animations[0].keyframes[0]: `target` is not three finite numbers",
            "a registered target's raw values are checked as the built-in ones are"
        );
        let still = r#"{"length":0,"loop":true,"animations":[{"bone":"a","target":"animfile:pulse",
            "keyframes":[{"timestamp":0,"target":[1,1,1],"interpolation":"animfile:step"}]}]}"#;
        assert_eq!(read(still).unwrap_err(), "`length` 0 is not positive");
    }

    /// A target and an interpolation are separate kinds: an id registered only as a target fails
    /// a file that names it as an interpolation, by name.
    #[test]
    fn a_registered_target_is_not_an_interpolation() {
        keep([(
            JsonAnimationType::Target(Identifier::parse("animkind:pulse").unwrap()),
            "pulser",
        )]);
        let as_interpolation = r#"{"length":1,"animations":[{"bone":"a","target":"animkind:pulse",
            "keyframes":[{"timestamp":0,"target":[1,1,1],"interpolation":"animkind:pulse"}]}]}"#;
        assert_eq!(
            read(as_interpolation).unwrap_err(),
            "animations[0].keyframes[0]: unknown interpolation animkind:pulse"
        );
    }

    /// Every load reads the files again: a nested path is part of the id, a broken file is left
    /// out while the others load, and an edited file plays its new pose on a mod's own model.
    #[test]
    fn a_mod_animation_loads_by_id_and_a_reload_reads_the_edited_file() {
        let source = |swing: &str| {
            ResourceSource::indexed(
                [
                    (
                        "assets/golems/neoforge/animations/entity/golem/swing.json",
                        swing.to_owned(),
                    ),
                    (
                        "assets/golems/neoforge/animations/entity/broken.json",
                        "{".to_owned(),
                    ),
                    ("assets/golems/other/swing.json", SWING.to_owned()),
                ]
                .into_iter()
                .map(|(name, text)| (name.to_owned(), Resource::bytes(text.into_bytes())))
                .collect(),
            )
        };
        let id = Identifier::parse("golems:golem/swing").unwrap();
        let model = ModelPart::new(
            Vec::new(),
            vec![(
                "arm".to_owned(),
                ModelPart::new(Vec::new(), Vec::new(), PartPose::ZERO),
            )],
            PartPose::ZERO,
        );
        let arm_x_rot = |animations: &Animations| {
            let holder = animations.get(&id).expect("the file loads by its id");
            let mut posed = model.clone();
            let mut state = AnimationState::new();
            state.start(0);
            // 10 ticks: 500 ms, halfway to the second keyframe.
            holder.bake(&model).unwrap().apply(&mut posed, &state, 10.0);
            posed.get_child("arm").unwrap().x_rot
        };
        let first = Animations::load(&source(SWING));
        assert_eq!(first.0.len(), 1, "only the good file under the folder");
        assert_eq!(arm_x_rot(&first), FRAC_PI_4);
        let edited = SWING.replace("[90, 0, -45]", "[180, 0, 0]");
        assert_eq!(arm_x_rot(&Animations::load(&source(&edited))), FRAC_PI_2);
    }
}
