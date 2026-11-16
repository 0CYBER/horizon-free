//! A guest's bundle click is heard by the players near them, not by the guest
//! (`BundleItem.playInsertSound` -> `Player.playSound` ->
//! `ServerLevel.playSeededSound` with `except = player`).
use super::*;
use crate::items::{ItemStack, Items};
use crate::lan::menu::Rules;

struct ItemRules(Items);
impl Rules for ItemRules {
    fn items(&self) -> &Items {
        &self.0
    }
    fn enchantments(&self) -> &crate::enchant::Enchantments {
        static ENCHANTMENTS: std::sync::LazyLock<crate::enchant::Enchantments> =
            std::sync::LazyLock::new(crate::enchant::tests::sample);
        &ENCHANTMENTS
    }
    fn max_stack(&self, item: &str) -> u32 {
        self.0.get(item).map_or(64, |i| i.max_stack)
    }
}

/// The prepared resources' items, or None (the test is skipped) without them.
fn rules() -> Option<ItemRules> {
    let root = std::env::var_os("HORIZON_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.local/horizon-client")
                .join(horizon_core::VANILLA_VERSION)
        });
    if !root.join("items.json").exists() {
        eprintln!("skipping resource-backed bundle check: {}", root.display());
        return None;
    }
    let registry = horizon_world::level::data::load_registry(&root).ok()?;
    let facts = crate::blocks::BlockFacts::load(&root, &registry).ok()?;
    Items::load(
        &root,
        &registry,
        &facts,
        &horizon_world::loot::PackOverlay::NONE,
        &horizon_world::item_defaults::read(&root).ok()?,
    )
    .ok()
    .map(ItemRules)
}

/// The sound packets a guest was sent: (event, volume, source).
fn sounds(rx: &std::sync::mpsc::Receiver<crate::lan::Out>) -> Vec<(String, f32, f32, i32)> {
    let mut out = Vec::new();
    while let Ok(sent) = rx.try_recv() {
        let crate::lan::Out::Packet(body) = sent else {
            continue;
        };
        let mut r = horizon_protocol::buf::Reader::new(&body);
        let id = r.varint().unwrap();
        if let Ok(C::Sound(p)) = C::decode(id, r.rest()) {
            let name = p.sound.1.map(|(name, _)| name).unwrap_or_default();
            out.push((name, p.volume, p.pitch, p.source));
        }
    }
    out
}

#[test]
fn a_guests_bundle_click_sounds_for_the_others_near_but_not_for_them() {
    let Some(rules) = rules() else { return };
    let (mut hub, bob_rx) = hub_with_a_guest();
    // Alex stands two blocks from Bob.
    let (tx, alex_rx) = std::sync::mpsc::channel();
    let mut alex = Guest::new(
        crate::lan::Joined {
            conn: 2,
            uuid: 2,
            name: "Alex".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: crate::lan::PlayerSender::new(tx, std::sync::Arc::default()),
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25566".parse().unwrap(),
            expected: false,
        },
        2,
        [2.5, 64.0, 0.5],
        8,
    );
    alex.placed = true;
    alex.dimension = "minecraft:overworld".into();
    hub.guests.push(alex);
    let mut world = World::new(horizon_world::registry::BlockStateId(0), 0);
    let click = |slot, button| play::ContainerClick {
        container: 0,
        state: 0,
        slot,
        button,
        input: 0,
        changed: Vec::new(),
        carried: None,
    };
    // Bob left-clicks four stone onto the bundle in his first hotbar slot
    // (inventory menu slot 36): `overrideOtherStackedOnMe` inserts them.
    hub.guests.update(0, |g| {
        g.inventory[0] = Some(ItemStack::new("minecraft:bundle", 1));
        g.carried = Some(ItemStack::new("minecraft:stone", 4));
    });
    while bob_rx.try_recv().is_ok() {}
    hub.container_click(1, &click(36, 0), &mut world, &rules);
    assert_eq!(hub.guests[0].carried, None, "the bundle took the stone");
    let heard = sounds(&alex_rx);
    assert_eq!(heard.len(), 1, "{heard:?}");
    let (name, volume, pitch, source) = heard[0].clone();
    assert_eq!(name, "minecraft:item.bundle.insert");
    assert_eq!(volume, 0.8);
    assert!((0.8..1.2).contains(&pitch), "{pitch}");
    assert_eq!(source, 7, "SoundSource.PLAYERS");
    assert_eq!(sounds(&bob_rx), [], "the clicker's client predicted it");
    // A right click with nothing carried takes one stack back out.
    hub.container_click(1, &click(36, 1), &mut world, &rules);
    let heard: Vec<String> = sounds(&alex_rx).into_iter().map(|s| s.0).collect();
    assert_eq!(heard, ["minecraft:item.bundle.remove_one"]);
    assert_eq!(sounds(&bob_rx), []);
}
