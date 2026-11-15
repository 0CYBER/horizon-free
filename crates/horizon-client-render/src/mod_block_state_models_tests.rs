use super::*;
use crate::{models::StateModels, resource_source::ResourceSource, sprites::Sprites};
use horizon_core::position::BlockPos;
use horizon_world::{
    DATA_VERSION,
    chunk::BlockState,
    registry::{BlockRegistry, BlockStateId},
};
use std::{collections::BTreeMap, fs, path::PathBuf};

fn pick(state: u32, model: &str) -> BlockStateModelPick {
    BlockStateModelPick::Model {
        state,
        model: model.into(),
        turns: [0; 3],
        uvlock: false,
    }
}

#[test]
fn picks_fill_each_states_slot_and_turns_wrap() {
    let picks = vec![
        BlockStateModelPick::Model {
            state: 2,
            model: "lamps:block/on".into(),
            turns: [4, 7, 255],
            uvlock: true,
        },
        pick(0, "block/off"),
    ];
    let models = picked("lamps:t", 3, picks).unwrap();
    assert_eq!(models[0].as_ref().unwrap().model, "minecraft:block/off");
    assert!(models[1].is_none());
    let on = models[2].as_ref().unwrap();
    assert_eq!((on.rotations, on.uv_lock), ([0, 3, 3], true));
}

/// Every malformed answer refuses the file with a reason, never a panic.
#[test]
fn a_malformed_answer_refuses_the_file_with_a_reason() {
    let cases = [
        (
            vec![pick(u32::MAX, "block/a")],
            "picked state 4294967295, but the block has 2 states",
        ),
        (
            vec![pick(2, "block/a")],
            "picked state 2, but the block has 2 states",
        ),
        (
            vec![pick(1, "block/a"), pick(1, "block/b")],
            "picked state 1 twice",
        ),
        (vec![pick(0, "Not A Model")], "picked model \"Not A Model\""),
        (Vec::new(), "picked no model"),
        (
            vec![
                pick(0, "block/a"),
                BlockStateModelPick::Refuse {
                    reason: "needs \"on\"".into(),
                },
            ],
            "refused it: needs \"on\"",
        ),
    ];
    for (picks, reason) in cases {
        let error = picked("lamps:t", 2, picks).unwrap_err();
        assert!(error.contains(reason), "{error:?} names {reason:?}");
        assert!(error.contains("lamps:t"), "{error:?} names the type");
    }
}

/// A lamp's definition type: state by state, `on` when `powered` is true, else `off`.
struct Lamps;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Lamps {
    fn register_block_state_models(
        &self,
        _: &RegistrationContext,
        event: &mut mod_api::RegisterBlockStateModelsEvent<'_>,
    ) {
        event.definitions.push("rendtest:by_power".into());
    }
    fn instantiate_block_state_definition(
        &self,
        _: &RegistrationContext,
        event: &mut mod_api::InstantiateBlockStateDefinitionEvent<'_>,
    ) {
        if event.definition_type != "rendtest:by_power" {
            return;
        }
        let (Some(Tag::String(on)), Some(Tag::String(off))) =
            (event.config.get("on"), event.config.get("off"))
        else {
            event.models.push(BlockStateModelPick::Refuse {
                reason: "needs \"on\" and \"off\"".into(),
            });
            return;
        };
        for (state, properties) in event.states.iter().enumerate() {
            let lit = properties.get("powered").is_some_and(|v| v == "true");
            let model = if lit { on } else { off };
            event
                .models
                .push(pick(state as u32, &model.to_string_lossy()));
        }
    }
}

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let fixture =
            Self(std::env::temp_dir().join(format!("horizon-bsm-{name}-{}", std::process::id())));
        let cube = r##"{"textures":{"all":"rendtest:block/lamp"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#all"}}}]}"##;
        for model in ["on", "off", "dim", "post"] {
            fixture.write(&format!("assets/rendtest/models/block/{model}.json"), cube);
        }
        // Vanilla's multipart beside the typed file.
        fixture.write(
            "assets/rendtest/blockstates/post.json",
            r#"{"multipart":[{"apply":{"model":"rendtest:block/post"}},{"apply":{"model":"rendtest:block/dim"}}]}"#,
        );
        fixture
    }
    fn write(&self, file: &str, json: &str) {
        let path = self.0.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, json).unwrap();
    }
    fn lamp(&self, json: &str) {
        self.write("assets/rendtest/blockstates/lamp.json", json);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// `rendtest:lamp` has two states (`powered` false, then true), `rendtest:post` one.
fn registry() -> BlockRegistry {
    let state = |name: &str, powered: Option<&str>| BlockState {
        name: name.into(),
        properties: powered
            .map(|p| BTreeMap::from([("powered".to_owned(), p.to_owned())]))
            .unwrap_or_default()
            .into(),
    };
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(vec![
                state("rendtest:lamp", Some("false")).to_nbt(),
                state("rendtest:lamp", Some("true")).to_nbt(),
                state("rendtest:post", None).to_nbt(),
            ]),
        ),
        (
            "defaults",
            Tag::compound([
                ("rendtest:lamp", Tag::Int(0)),
                ("rendtest:post", Tag::Int(2)),
            ]),
        ),
    ]))
    .unwrap()
}

/// The models each state of the registry draws, as a load of `fixture` bakes them: what a
/// resource load or reload builds.
fn load(fixture: &Fixture) -> Vec<Vec<String>> {
    let facts: BTreeMap<String, serde_json::Value> = serde_json::from_str(
        r#"{"rendtest:lamp":{"schema":{"powered":{"named":["false","true"]}},
            "states":[{"powered":"false"},{"powered":"true"}]},
            "rendtest:post":{"schema":{},"states":[{}]}}"#,
    )
    .unwrap();
    let source = ResourceSource::directory(fixture.0.clone());
    let mut sprites = Sprites::new(source.clone());
    let catalog = StateModels::from_facts(
        &source,
        &registry(),
        &["rendtest:lamp", "rendtest:post"],
        &mut sprites,
        &facts,
    )
    .unwrap();
    (0..3)
        .map(|state| {
            catalog
                .collect(BlockStateId(state), BlockPos::new(0, 0, 0))
                .map(|models| models.iter().map(|(v, _)| v.model.clone()).collect())
                .unwrap_or_default()
        })
        .collect()
}

fn names(models: &[&str]) -> Vec<String> {
    models.iter().map(|m| (*m).to_owned()).collect()
}

/// A two-state block picks each state's model through the mod's type; a reload over the edited
/// file picks again; a vanilla multipart block beside it keeps its parts; a file the type
/// refuses, or one naming a type nobody registered, leaves the block without a model.
#[test]
fn a_registered_type_picks_each_states_model_and_again_on_reload() {
    use mod_api::__private::game::RegistrationContextNew;
    let fixture = Fixture::new("reload");
    let post = names(&["rendtest:block/post", "rendtest:block/dim"]);
    let none = Vec::new();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("lamps", Box::new(Lamps))].into();
    mod_platform::testing::with_mods(mods, || {
        let mut ids = Vec::new();
        mod_api::dispatch_register_block_state_models(&RegistrationContext::new(), &mut ids);
        assert_eq!(register(ids), ["rendtest:by_power"]);
        fixture.lamp(
            r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/on","off":"rendtest:block/off"}"#,
        );
        assert_eq!(
            load(&fixture),
            [
                names(&["rendtest:block/off"]),
                names(&["rendtest:block/on"]),
                post.clone()
            ]
        );
        // A pack edit, then a reload: the powered lamp is dim now.
        fixture.lamp(
            r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/dim","off":"rendtest:block/off"}"#,
        );
        assert_eq!(
            load(&fixture),
            [
                names(&["rendtest:block/off"]),
                names(&["rendtest:block/dim"]),
                post.clone()
            ]
        );
        // Malformed for the type: refused.
        fixture
            .lamp(r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/on"}"#);
        assert_eq!(load(&fixture), [none.clone(), none.clone(), post.clone()]);
        // A type no mod registered: refused.
        fixture.lamp(
            r#"{"neoforge:definition_type":"rendtest:never","on":"rendtest:block/on","off":"rendtest:block/off"}"#,
        );
        assert_eq!(load(&fixture), [none.clone(), none.clone(), post.clone()]);
    });
    // Registered, but with no mod loaded to answer: no picks, refused.
    fixture.lamp(
        r#"{"neoforge:definition_type":"rendtest:by_power","on":"rendtest:block/on","off":"rendtest:block/off"}"#,
    );
    mod_platform::testing::with_mods(Vec::new().into(), || {
        assert_eq!(load(&fixture), [none.clone(), none.clone(), post.clone()]);
    });
}

/// A pack's typed layer over vanilla's variants replaces the states it picks; the others keep the
/// layer below, as stacked vanilla definitions do.
#[test]
fn a_typed_layer_over_vanilla_replaces_only_the_states_it_picks() {
    struct PoweredOnly;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for PoweredOnly {
        fn instantiate_block_state_definition(
            &self,
            _: &RegistrationContext,
            event: &mut mod_api::InstantiateBlockStateDefinitionEvent<'_>,
        ) {
            if event.definition_type == "rendtest:powered_only" {
                event.models.push(pick(1, "rendtest:block/on"));
            }
        }
    }
    let fixture = Fixture::new("layers");
    let stacked = serde_json::json!({ crate::packs::BLOCKSTATE_LAYERS: [
        {"variants":{"":{"model":"rendtest:block/off"}}},
        {"neoforge:definition_type":"rendtest:powered_only"}
    ]});
    fixture.lamp(&stacked.to_string());
    register(vec!["rendtest:powered_only".into()]);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("on", Box::new(PoweredOnly))].into();
    mod_platform::testing::with_mods(mods, || {
        let picked = load(&fixture);
        assert_eq!(picked[0], ["rendtest:block/off"]);
        assert_eq!(picked[1], ["rendtest:block/on"]);
    });
}
