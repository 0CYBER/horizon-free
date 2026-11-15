use crate::game::native_entity_refs::tests::{run_fresh_cases, with_client_game};
use horizon_client::{
    advancements::{Progress, Tree},
    hud::{GuiScale, HudBatch},
    lan::{ServerAdvancements, data::SyncData},
};
use horizon_core::nbt::Tag;
use horizon_protocol::{
    buf::{Codec, Reader},
    item::{Component, WirePatch, component, custom},
    packets::play::UpdateAdvancements,
};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../horizon-client/tests/fixtures/advancement-icons")
}

fn batch() -> HudBatch {
    HudBatch::new(
        [640, 480],
        GuiScale {
            requested: 1,
            unicode: false,
        },
    )
}

fn glint_vertices(batch: &HudBatch, x: f32, y: f32) -> usize {
    batch
        .vertices
        .iter()
        .filter(|v| {
            (2.0..4.0).contains(&v.uv[0])
                && (x..=x + 16.0).contains(&v.pos[0])
                && (y..=y + 16.0).contains(&v.pos[1])
        })
        .count()
}

fn assert_icon(batch: &HudBatch, x: f32, y: f32, foil: bool, damaged: bool) {
    assert_eq!(glint_vertices(batch, x, y) > 0, foil, "glint at ({x}, {y})");
    // Count 3 is drawn in the icon's lower-right corner, not a default count 1.
    assert!(
        batch.vertices.iter().any(|v| {
            v.text == 1
                && (x + 9.0..=x + 17.0).contains(&v.pos[0])
                && (y + 9.0..=y + 18.0).contains(&v.pos[1])
        }),
        "count decoration at ({x}, {y})"
    );
    // A wooden pickaxe has 59 durability: damage 7 leaves an 11-pixel bar.
    let bar = batch.vertices.windows(6).any(|vertices| {
        let min_x = vertices
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::INFINITY, f32::min);
        let max_x = vertices
            .iter()
            .map(|v| v.pos[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::INFINITY, f32::min);
        let max_y = vertices
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::NEG_INFINITY, f32::max);
        [min_x, min_y, max_x, max_y] == [x + 2.0, y + 13.0, x + 13.0, y + 14.0]
    });
    assert_eq!(bar, damaged, "damage decoration at ({x}, {y})");
}

#[test]
#[ignore = "daily"]
fn advancement_icons_reach_local_and_remote_ui_surfaces() {
    const FLAG: &str = "HORIZON_ADVANCEMENT_ICONS_CHILD";
    if run_fresh_cases(
        "game::advancements::icons::tests::advancement_icons_reach_local_and_remote_ui_surfaces",
        FLAG,
        &["local", "remote", "refused", "invalid"],
    ) {
        return;
    }
    let case = std::env::var(FLAG).unwrap();
    let refused = matches!(case.as_str(), "refused" | "invalid");
    if refused {
        // Process-isolated: never changes another test's installed codec table.
        custom::install(1_000_000, vec!["horizon_test:refused".into()]).unwrap();
    }
    with_client_game(|game| {
        let tree = Tree::load(&fixture().join("data"));
        if case == "local" {
            game.adv.tree = Arc::new(tree);
            game.adv.loaded = true;
            let tree = game.adv.tree.clone();
            for a in &tree.list {
                game.adv
                    .progress
                    .grant(&a.id, "done", "2026-01-01 00:00:00 +0000");
                game.award_definition(a);
            }
        } else {
            game.init_remote_for_replay().unwrap();
            let mut sync = SyncData::empty();
            sync.builtin = (*game.remote.as_ref().unwrap().builtin).clone();
            sync.advancements = ServerAdvancements::load(
                &fixture(),
                &horizon_world::loot::PackOverlay::NONE,
                Default::default(),
                Default::default(),
            );
            let mut visible = Vec::new();
            let mut progress = Progress::default();
            for a in &tree.list {
                progress.grant(&a.id, "done", "2026-01-01 00:00:00 +0000");
            }
            let bytes = sync
                .advancements
                .update(&sync, &progress, &mut visible, &BTreeSet::new(), true)
                .unwrap();
            let mut packet = UpdateAdvancements::read(&mut Reader::new(&bytes)).unwrap();
            assert!(packet.reset);
            assert_eq!(packet.added.len(), 2);
            for a in &mut packet.added {
                let icon = &mut a.display.as_mut().unwrap().icon;
                assert_eq!((icon.count, icon.damage()), (3, 7));
                if refused {
                    if case == "refused" {
                        icon.added.push((1_000_000, Component::Nbt(Tag::Int(9))));
                        assert!(!custom::permitted(icon, false).unwrap());
                    } else {
                        icon.added
                            .push((component::CONTAINER, Component::Raw(vec![])));
                        assert!(custom::permitted(icon, false).is_err());
                    }
                    icon.removed.push(component::CUSTOM_NAME);
                    icon.wire = WirePatch(Some(Arc::from([255u8])));
                }
            }
            game.remote_advancements(packet);
            assert!(game.adv.toasts.is_empty(), "initial reset is quiet");
            let changed: BTreeSet<_> = (0..tree.list.len()).collect();
            let bytes = sync
                .advancements
                .update(&sync, &progress, &mut visible, &changed, true)
                .unwrap();
            let packet = UpdateAdvancements::read(&mut Reader::new(&bytes)).unwrap();
            assert!(!packet.reset);
            assert!(
                packet.added.is_empty(),
                "progress reuses the already converted icon"
            );
            game.remote_advancements(packet);
        }
        assert_eq!(
            game.adv.tree.list.len(),
            2,
            "refused icons keep both advancements"
        );
        assert_eq!(game.adv.toasts.len(), 2);
        for (id, foil) in [
            ("horizon_test:glint", !refused),
            ("horizon_test:plain", false),
        ] {
            let icon = &game
                .adv
                .tree
                .get(id)
                .unwrap()
                .display
                .as_ref()
                .unwrap()
                .icon;
            assert_eq!(
                (&*icon.item, icon.count, icon.damage),
                ("minecraft:wooden_pickaxe", 3, if refused { 0 } else { 7 })
            );
            assert_eq!(icon.has_foil(None), foil);
            if refused {
                assert!(
                    icon.data().is_none(),
                    "additions, removals and cached wire are stripped"
                );
            }
        }
        let toast = &mut game.adv.toasts[0];
        assert_eq!(
            (&*toast.icon.item, toast.icon.count, toast.icon.damage),
            ("minecraft:wooden_pickaxe", 3, if refused { 0 } else { 7 })
        );
        assert_eq!(toast.icon.has_foil(None), !refused);
        toast.slide.portion = 1.0;
        game.glint_frame = horizon_client::item_render::GlintFrame {
            millis: 1000.0,
            speed: 0.5,
            alpha: 0.75,
            sprite: true,
        };
        let mut toast_batch = batch();
        game.draw_toast(&mut toast_batch, 0, 0.0, 1.0);
        assert_icon(&toast_batch, 488.0, 8.0, !refused, !refused);

        game.open_advancements(false);
        game.cursor = (0.0, 0.0);
        let mut screen = batch();
        game.draw_advancements(&mut screen);
        assert_icon(&screen, 314.0, 236.0, !refused, !refused); // widget
        assert_icon(&screen, 200.0, 151.0, !refused, !refused); // glint tab
        assert_icon(&screen, 232.0, 151.0, false, !refused); // plain control tab
        let widget_glint = glint_vertices(&screen, 314.0, 236.0);
        game.cursor = (320.0, 240.0);
        let mut hover = batch();
        game.draw_advancements(&mut hover);
        assert_icon(&hover, 314.0, 236.0, !refused, !refused);
        assert_eq!(
            glint_vertices(&hover, 314.0, 236.0),
            widget_glint * 2,
            "hover submits the full icon again over its widget"
        );
    });
}
