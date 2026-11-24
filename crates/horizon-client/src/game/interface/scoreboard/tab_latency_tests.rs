use super::Game;
use crate::game::native_entity_refs::tests::with_client_game;
use horizon_client::{
    held::Source,
    hud::HudBatch,
    hud_layout::abgr,
    tab_list::{LATENCY_WIDTH, MAX_LATENCY_WIDTH, ping_sprite},
};
use horizon_core::nbt::Tag;
use horizon_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};
use mod_api::{ClientHost, Flow, Hooks, ModEntry, TabListLatencyDrawEvent, TabListLatencyWidthEvent};
use std::sync::{Arc, Mutex};
use winit::keyboard::KeyCode;

type Rows = Arc<Mutex<Vec<(u128, String, i32, [i32; 4])>>>;

struct NumericPing(Rows);

fn color(latency: i32) -> u32 {
    0xff36_8000 | latency as u32
}

#[mod_api::__subscriptions]
impl Hooks for NumericPing {
    fn tab_list_latency_width(&self, _: &dyn ClientHost, event: &mut TabListLatencyWidthEvent) {
        assert_eq!(event.width, LATENCY_WIDTH);
        event.width = i32::MAX;
    }

    fn tab_list_latency_draw(
        &self,
        _: &dyn ClientHost,
        event: &mut TabListLatencyDrawEvent<'_>,
    ) -> Flow {
        self.0.lock().unwrap().push((
            event.player_uuid,
            event.player_name.to_owned(),
            event.latency_ms,
            event.bounds,
        ));
        let [x, y, right, bottom] = event.bounds.map(|n| n as f32);
        event.draw.fill(
            [x - 20.0, y - 20.0, right - x + 40.0, bottom - y + 40.0],
            color(event.latency_ms),
        );
        event.draw.text(
            &format!("{} ms", event.latency_ms),
            [x + 2.0, y],
            0xffff_ffff,
            false,
        );
        Flow::Cancel
    }
}

fn draw(game: &mut Game<'_>) -> HudBatch {
    let mut batch = HudBatch::scratch([1920, 1080]);
    game.gui_layer("minecraft:tab_list", &mut batch, |game, batch| {
        game.draw_tab_list(batch);
    })
    .unwrap();
    batch
}

fn bounds(vertices: impl IntoIterator<Item = [f32; 2]>) -> [f32; 4] {
    vertices.into_iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |[left, top, right, bottom], [x, y]| [left.min(x), top.min(y), right.max(x), bottom.max(y)],
    )
}

#[test]
#[ignore = "daily"]
fn live_tab_latency_hooks_keep_scores_and_replace_clipped_ping_icons() {
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "LatencyFixture").unwrap();
        game.screen = crate::game::Screen::Playing;
        game.keys.press(KeyCode::Tab, Source::Keyboard);
        game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
            actions: 1 | 8 | 16, // add, update listed, update latency
            entries: [(2, "Zara", 210), (1, "Alice", 47)]
                .map(|(uuid, name, latency)| play::PlayerInfo {
                    uuid,
                    name: Some(name.into()),
                    listed: Some(true),
                    latency: Some(latency),
                    ..Default::default()
                })
                .into(),
        }))
        .unwrap();
        game.handle_packet(C::SetObjective(play::SetObjective {
            name: "wins".into(),
            method: 0,
            display: Some((Tag::String("Wins".into()), 0, None)),
        }))
        .unwrap();
        game.handle_packet(C::SetDisplayObjective(play::SetDisplayObjective {
            slot: 0,
            objective: "wins".into(),
        }))
        .unwrap();
        for (owner, score) in [("Alice", 1234), ("Zara", 5678)] {
            game.handle_packet(C::SetScore(play::SetScore {
                owner: owner.into(),
                objective: "wins".into(),
                score,
                display: None,
                number_format: None,
            }))
            .unwrap();
        }

        let stock = draw(game);
        let rows = Rows::default();
        let custom = mod_platform::testing::with_mods(
            vec![ModEntry::new("numeric_ping", Box::new(NumericPing(Arc::clone(&rows))))].into(),
            || draw(game),
        );
        let restored = draw(game);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&stock.vertices),
            bytemuck::cast_slice::<_, u8>(&restored.vertices),
        );
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 2);
        let stock_row = bounds(stock.vertices[6..12].iter().map(|v| v.pos));
        let custom_row = bounds(custom.vertices[6..12].iter().map(|v| v.pos));
        assert_eq!(
            (custom_row[2] - custom_row[0]) - (stock_row[2] - stock_row[0]),
            (MAX_LATENCY_WIDTH - LATENCY_WIDTH) as f32,
        );

        for ((uuid, name, latency, slot), (id, profile, ms, y)) in rows
            .iter()
            .zip([(1, "Alice", 47, 10), (2, "Zara", 210, 19)])
        {
            assert_eq!((*uuid, name.as_str(), *latency, slot[1]), (id, profile, ms, y));
            let [left, _, right, bottom] = *slot;
            assert_eq!(right - left, MAX_LATENCY_WIDTH - 2);
            assert_eq!(bottom, y + 8);
            let painted = custom
                .vertices
                .iter()
                .filter(|v| v.color == abgr(color(ms)))
                .map(|v| v.pos);
            assert_eq!(bounds(painted), slot.map(|n| n as f32));

            let sprite = format!("gui/sprites/{}", ping_sprite(ms));
            let region = game.gui.atlas.sprites[sprite.as_str()];
            let has_ping = |batch: &HudBatch| {
                batch.vertices.iter().any(|v| v.uv == [region.u0, region.v0])
            };
            assert!(has_ping(&stock), "stock row uses {sprite}");
            assert!(!has_ping(&custom), "the mod canceled {sprite}");
        }

        let scores = |batch: &HudBatch, row_left: f32| {
            batch
                .vertices
                .iter()
                .filter(|v| v.text != 0 && v.color == abgr(0xffff_ff55))
                .map(|v| ([v.pos[0] - row_left, v.pos[1]], v.uv))
                .collect::<Vec<_>>()
        };
        let stock_scores = scores(&stock, stock_row[0]);
        assert!(!stock_scores.is_empty());
        assert_eq!(stock_scores, scores(&custom, custom_row[0]));
    });
}
