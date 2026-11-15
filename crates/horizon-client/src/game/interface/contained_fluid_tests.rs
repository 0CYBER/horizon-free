//! `contained_fluid`: a mod's own container item says which fluid a stack holds, and its
//! `neoforge:fluid_container` model draws it as a held stack of that fluid. Run alone with
//! prepared resources: installs process-wide trait mods.
use horizon_assets::item_model::{
    ClientItem, Context, DisplayContext, ItemModels, Layer, LayerContent, StackView,
};
use mod_api::{
    ClientHost, ContainedFluidEvent, FluidKey, Hooks, ItemPart, Registrar, RegistrationContext,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

/// A canister's model: a bucket's base, the held fluid through the bucket's fluid mask, and
/// no fluid layer while it holds none.
const MODEL: &str = r#"{"type":"neoforge:fluid_container","fluid":"minecraft:empty",
    "textures":{"base":"minecraft:item/bucket","fluid":"neoforge:item/mask/bucket_fluid"}}"#;
/// [`MODEL`] declaring water as what it draws while it holds none.
const WATER_FALLBACK: &str = r#"{"type":"neoforge:fluid_container","fluid":"minecraft:water",
    "textures":{"base":"minecraft:item/bucket","fluid":"neoforge:item/mask/bucket_fluid"}}"#;

/// What the canister's part answers.
#[derive(Clone, Copy, Debug)]
enum Answer {
    Nothing,
    Water,
    /// A tank-only fluid of the mod's own with no look: an item cannot draw it.
    Sludge,
}

static WATER: OnceLock<FluidKey> = OnceLock::new();
static SLUDGE: OnceLock<FluidKey> = OnceLock::new();

#[derive(Clone)]
struct Canister {
    answer: Arc<Mutex<Answer>>,
    /// The items whose stacks the part was asked about.
    asked: Arc<Mutex<Vec<String>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Canister {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        use crate::game::native_entity_refs::tests::record_key;
        record_key(&WATER, r.fluid_key("minecraft:water"));
        record_key(&SLUDGE, r.declare_fluid("sludge"));
        // Attached to vanilla items too, which must never ask it.
        for item in [
            "tank:canister",
            "minecraft:water_bucket",
            "minecraft:bucket",
        ] {
            let key = r.item_key(item);
            r.item_part(key, self.clone());
        }
    }
}
#[mod_api::horizon_part]
impl ItemPart for Canister {
    fn contained_fluid(&self, _: &dyn ClientHost, event: &mut ContainedFluidEvent<'_>) {
        let item = mod_api::__private::item_key_id(event.stack.item()).unwrap();
        self.asked.lock().unwrap().push(item.to_owned());
        *event.fluid = match *self.answer.lock().unwrap() {
            Answer::Nothing => None,
            Answer::Water => WATER.get().copied(),
            Answer::Sludge => SLUDGE.get().copied(),
        };
    }
}

/// A resource root with the mod's content: `tank:canister`, a bucket in all but name.
fn resource_root(scratch: &std::path::Path) -> std::path::PathBuf {
    let base = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let module = scratch.join("module");
    std::fs::create_dir_all(module.join("data/tank/horizon/item")).unwrap();
    std::fs::write(
        module.join("horizon-mod.toml"),
        "id = 'tank'\nname = 'Canister fixture'\nversion = '1.0'\napi = '0.2'\nside = 'both'\ncapabilities = ['registries']\n",
    )
    .unwrap();
    std::fs::write(
        module.join("data/tank/horizon/item/canister.json"),
        r#"{"template":"minecraft:bucket"}"#,
    )
    .unwrap();
    let package = horizon_mod_package::Package::read(&module).unwrap();
    let content = horizon_mod_package::Content::read(&module, &package.manifest).unwrap();
    let root = scratch.join("resources");
    horizon_client::mod_content::build(
        &base,
        std::path::Path::new(""),
        &[horizon_client::mod_content::ContentMod {
            id: "tank".into(),
            content,
            jars: vec![],
        }],
        &root,
        &horizon_mod_package::enabled::EnabledMods::new([("tank".into(), mod_api::Side::Both)]),
    )
    .unwrap();
    root
}

/// The layers `item` draws under [`MODEL`], asking the game.
fn drawn(game: &crate::game::Game<'_>, item: &str) -> Vec<Layer> {
    drawn_with(game, item, MODEL)
}
/// The layers `item` draws under the fluid container `model`, asking the game.
fn drawn_with(game: &crate::game::Game<'_>, item: &str, model: &str) -> Vec<Layer> {
    let mut models = ItemModels::default();
    models.definitions.insert(
        item.into(),
        ClientItem::from_json(&format!(r#"{{"model":{model}}}"#)).unwrap(),
    );
    Arc::make_mut(&mut models.prototypes).insert(
        item.into(),
        BTreeMap::from([(
            "minecraft:item_model".into(),
            serde_json::Value::String(item.into()),
        )]),
    );
    let context = Context {
        display: DisplayContext::Gui,
        owner: None,
        level: None,
        seed: 0,
    };
    models
        .resolve(&StackView::new(item, 1), &context, &game.item_env())
        .layers
}

#[test]
#[ignore = "daily"]
fn a_mod_container_draws_the_fluid_its_part_reports() {
    crate::game::in_own_process(
        "game::contained_fluid_tests::a_mod_container_draws_the_fluid_its_part_reports",
        a_mod_container_draws_the_fluid_its_part_reports_isolated,
    );
}

fn a_mod_container_draws_the_fluid_its_part_reports_isolated() {
    let part = Canister {
        answer: Arc::new(Mutex::new(Answer::Nothing)),
        asked: Arc::new(Mutex::new(Vec::new())),
    };
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("tank", Box::new(part.clone()))].into();
    let scratch = tempfile::tempdir().unwrap();
    let root = resource_root(scratch.path());
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::game::native_entity_refs::tests::with_game_at(root, |game| {
            let answer = |a| *part.answer.lock().unwrap() = a;
            let water_still = LayerContent::Sprite {
                sprite: "minecraft:block/water_still".into(),
                mask: Some("neoforge:item/mask/bucket_fluid".into()),
            };

            // Reporting water draws what a water bucket draws: its still sprite through the
            // mask, tinted as a held water stack (not white, the empty container's tint).
            answer(Answer::Water);
            let canister = drawn(game, "tank:canister");
            let bucket = drawn(game, "minecraft:water_bucket");
            assert_eq!(canister.len(), 2, "{canister:?}");
            assert_eq!(canister[1].content, water_still);
            assert_eq!(canister[1].tints, bucket[1].tints);
            assert_ne!(canister[1].tints, vec![-1]);

            // Reporting nothing draws the empty container: the base alone.
            answer(Answer::Nothing);
            let empty = drawn(game, "tank:canister");
            assert_eq!(empty.len(), 1, "{empty:?}");

            // A registered fluid with no look in an item draws the missing sprite through the
            // mask, untinted, so the mistake shows.
            answer(Answer::Sludge);
            let sludge = drawn(game, "tank:canister");
            assert_eq!(sludge.len(), 2, "{sludge:?}");
            assert_eq!(
                sludge[1].content,
                LayerContent::Sprite {
                    sprite: "minecraft:missingno".into(),
                    mask: Some("neoforge:item/mask/bucket_fluid".into()),
                }
            );
            assert_eq!(sludge[1].tints, vec![-1]);

            // Holding none under a model that declares water draws that water untinted (white).
            answer(Answer::Nothing);
            let fallback = drawn_with(game, "tank:canister", WATER_FALLBACK);
            assert_eq!(fallback.len(), 2, "{fallback:?}");
            assert_eq!(fallback[1].content, water_still);
            assert_eq!(fallback[1].tints, vec![-1]);

            // Vanilla's items never ask a part, even one attached to them: the water bucket
            // keeps vanilla's answer and the empty bucket stays empty.
            answer(Answer::Water);
            assert_eq!(drawn(game, "minecraft:bucket").len(), 1);
            assert_eq!(
                drawn(game, "minecraft:water_bucket")[1].content,
                water_still
            );
            let asked = part.asked.lock().unwrap().clone();
            assert!(
                asked.iter().all(|item| item == "tank:canister"),
                "{asked:?}"
            );
            assert_eq!(asked.len(), 4);
        });
    });
}
