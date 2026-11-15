/// Install parts that answer a block's sound type: glass answers `mymod:block.x.<sound>` at
/// volume 2.0 and pitch 1.5 to every entity and cause, and white wool the same to a player
/// only. Red wool answers a NaN volume, and blue wool events with no namespace. A snow layer
/// answers as glass does, but only when asked at y 64.
pub fn sound_type_game_thread() {
    struct Answer {
        volume: f32,
        prefix: &'static str,
        players_only: bool,
        at_y: Option<i32>,
    }
    #[mod_api::horizon_part]
    impl BlockPart for Answer {
        fn sound_type(&self, _: &dyn LoopContext, event: &mut SoundTypeEvent<'_>) {
            let player = |(_, kind): (u32, &str)| kind == "minecraft:player";
            if self.players_only && !event.entity.is_some_and(player)
                || self.at_y.is_some_and(|y| event.pos.y != y)
            {
                return;
            }
            let event_id =
                |sound: &str| -> std::sync::Arc<str> { format!("{}{sound}", self.prefix).into() };
            *event.sound = BlockSoundType {
                volume: self.volume,
                pitch: 1.5,
                break_sound: event_id("break"),
                step: event_id("step"),
                place: event_id("place"),
                hit: event_id("hit"),
                fall: event_id("fall"),
            };
        }
    }
    struct SoundTypeMod;
    #[mod_api::__subscriptions]
    impl Hooks for SoundTypeMod {
        fn register(&self, r: &mut dyn Registrar, _: &crate::RegistrationContext) {
            let part = |volume, prefix, players_only| Answer {
                volume,
                prefix,
                players_only,
                at_y: None,
            };
            for (block, answer) in [
                ("minecraft:glass", part(2.0, "mymod:block.x.", false)),
                ("minecraft:white_wool", part(2.0, "mymod:block.x.", true)),
                (
                    "minecraft:red_wool",
                    part(f32::NAN, "mymod:block.x.", false),
                ),
                ("minecraft:blue_wool", part(2.0, "block.x.", false)),
                (
                    "minecraft:snow",
                    Answer {
                        at_y: Some(64),
                        ..part(2.0, "mymod:block.x.", false)
                    },
                ),
            ] {
                let key = r.block_key(block);
                r.block_part(key, answer);
            }
        }
    }
    let mods: crate::Mods = vec![ModEntry::new("sound_type_test", Box::new(SoundTypeMod))].into();
    crate::__private::install(mods.clone());
    install_parts(&mods, &crate::parts::tests::rule_schema(), |_| false).unwrap();
    mark_game_thread();
}
