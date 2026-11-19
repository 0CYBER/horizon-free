//! The old `ServerPlayer` retained across `restoreFrom`, independently of its replacement.
use super::Game;
use mod_api::__private::game::Fields;
use mod_api::{Identifier, LevelRef, OriginalPlayerRef};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-player attachment state and the stack of originals retained during clone hooks.
#[derive(Default)]
pub(super) struct PlayerState {
    pub(super) attachments: horizon_mod_runtime::attachments::ResidentAttachments,
    pub(super) originals: Vec<OriginalPlayerSnapshot>,
}

pub(super) struct OriginalPlayerSnapshot {
    token: u64,
    uuid: u128,
    position: [f64; 3],
    dimension: String,
    level: Option<LevelRef>,
    inventory: Vec<Option<horizon_items::items::ItemStack>>,
    armor: [Option<horizon_items::items::ItemStack>; 4],
    offhand: Option<horizon_items::items::ItemStack>,
    health: f32,
    experience: mod_api::PlayerExperience,
    effects: Vec<mod_api::ActiveEffect>,
    pub(super) attachments: mod_api::Attachments,
}

impl OriginalPlayerSnapshot {
    pub(super) fn token(&self) -> u64 {
        self.token
    }
    pub(super) fn uuid(&self) -> u128 {
        self.uuid
    }
}

impl Game<'_> {
    /// `ServerPlayer.restoreFrom`: retain the old object before replacing any data.
    pub(in crate::game) fn capture_original_player(
        &self,
        conn: u32,
    ) -> Option<OriginalPlayerSnapshot> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if !mod_api::__private::listened(mod_api::HookId::PlayerClone) {
            return None;
        }
        let guest = self.lan.as_ref()?.hub.guest(conn)?;
        let token = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .expect("original player identity exhausted");
        Some(OriginalPlayerSnapshot {
            token,
            uuid: guest.uuid,
            position: guest.position,
            dimension: guest.dimension.clone(),
            level: self.level(&guest.dimension),
            inventory: guest.inventory.clone(),
            armor: guest.armor.clone(),
            offhand: guest.offhand.clone(),
            health: guest.health,
            experience: mod_api::PlayerExperience::from(
                mod_api::__private::built::PlayerExperience {
                    level: guest.xp_level,
                    progress: guest.xp_progress,
                    total: guest.xp_total,
                },
            ),
            effects: guest
                .effects
                .list
                .iter()
                .map(super::native_living_effects::active_effect)
                .collect(),
            attachments: (*guest.attachments).clone(),
        })
    }

    pub(in crate::game) fn with_original_player(
        &mut self,
        original: OriginalPlayerSnapshot,
        call: impl FnOnce(&mut Self, OriginalPlayerRef),
    ) {
        let handle = mod_api::__private::game::original_player_ref(original.token);
        self.player_state.originals.push(original);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(self, handle)));
        self.player_state.originals.pop();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }

    fn original_player_snapshot(
        &self,
        original: OriginalPlayerRef,
    ) -> Option<&OriginalPlayerSnapshot> {
        self.player_state
            .originals
            .iter()
            .rev()
            .find(|snapshot| snapshot.token == original.fields())
    }

    pub(super) fn original_player_uuid(&self, original_player: OriginalPlayerRef) -> Option<u128> {
        Some(self.original_player_snapshot(original_player)?.uuid)
    }
    pub(super) fn original_player_position(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<[f64; 3]> {
        Some(self.original_player_snapshot(original_player)?.position)
    }
    pub(super) fn original_player_dimension(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<Identifier> {
        Identifier::parse(&self.original_player_snapshot(original_player)?.dimension).ok()
    }
    pub(super) fn original_player_level(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<LevelRef> {
        self.original_player_snapshot(original_player)?.level
    }
    pub(super) fn original_player_health(&self, original_player: OriginalPlayerRef) -> Option<f32> {
        Some(self.original_player_snapshot(original_player)?.health)
    }
    pub(super) fn original_player_experience(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<mod_api::PlayerExperience> {
        Some(self.original_player_snapshot(original_player)?.experience)
    }
    pub(super) fn original_player_effects(
        &self,
        original_player: OriginalPlayerRef,
    ) -> Option<Vec<mod_api::ActiveEffect>> {
        Some(
            self.original_player_snapshot(original_player)?
                .effects
                .clone(),
        )
    }
    pub(super) fn original_player_inventory_slot(
        &self,
        original_player: OriginalPlayerRef,
        slot: i32,
    ) -> Option<horizon_items::items::ItemStack> {
        let snapshot = self.original_player_snapshot(original_player)?;
        let item = match slot {
            0..=35 => snapshot.inventory.get(slot as usize)?.as_ref(),
            100..=103 => snapshot.armor[(103 - slot) as usize].as_ref(),
            -106 => snapshot.offhand.as_ref(),
            _ => None,
        }?;
        Some(item.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Dim, native_guest_turn_tests::with_guests};
    use mod_api::{AttachmentType, Holder, Hooks, PlayerCloneEvent, ServerHost};
    use std::sync::{Arc, Mutex};

    const VALUE: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "unsaved",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    const RETAINED: AttachmentType<i32> = AttachmentType {
        namespace: "clone",
        name: "retained",
        default: || 0,
        codec: Some(mod_api::AttachmentCodec {
            encode: |value| Ok([("value".into(), horizon_core::nbt::Tag::Int(*value))].into()),
            decode: |value| match value.get(&horizon_core::nbt::JavaString::from("value")) {
                Some(horizon_core::nbt::Tag::Int(value)) => Ok(*value),
                _ => Err(mod_api::AttachmentError::Decode),
            },
        }),
        copy_on_death: true,
        sync: false,
    };

    const SAVED: AttachmentType<i32> = AttachmentType {
        name: "saved",
        copy_on_death: false,
        ..RETAINED
    };

    fn wasm_reads(
        host: &mut dyn ServerHost,
        event: &mut PlayerCloneEvent<'_>,
        death: bool,
        keep: bool,
    ) -> horizon_mod_wasm::WasmMod {
        use mod_api::wire::Writer;
        let escape = |bytes: &[u8]| {
            bytes
                .iter()
                .map(|b| format!("\\{b:02x}"))
                .collect::<String>()
        };
        let mut data = String::new();
        let mut body = String::new();
        let mut offset = 4096;
        let mut check = |owner: &str, name: &str, input: Writer, expected: Writer| {
            let id = horizon_mod_wasm::fixture::import_id(owner, name).unwrap();
            data.push_str(&format!(
                "(data (i32.const {offset}) \"{}\")",
                escape(input.bytes())
            ));
            // The original is read from the guest event, never supplied as fixture data.
            let token_offset = offset + if owner == "AttachmentStore" { 4 } else { 0 };
            body.push_str(&format!(
                "i32.const {token_offset} global.get $original i64.store "
            ));
            body.push_str(&format!("i32.const {id} i32.const {offset} i32.const {} i32.const 32768 i32.const 8192 call $host drop ", input.bytes().len()));
            for (index, byte) in expected.bytes().iter().enumerate() {
                body.push_str(&format!(
                    "i32.const {} i32.load8_u i32.const {byte} i32.ne if unreachable end ",
                    32768 + index
                ));
            }
            offset += 2048;
        };
        let input = || {
            let mut w = Writer::new();
            w.put(&0u64);
            w
        };
        let mut expected = Writer::new();
        expected.put(&Some([7.0f64, 64.0, -9.0]));
        check("Backend", "original_player_position", input(), expected);
        let mut expected = Writer::new();
        expected.put(&Some(if death { 0.0f32 } else { 13.0 }));
        check("Backend", "original_player_health", input(), expected);
        let mut expected = Writer::new();
        expected.put(&Some(mod_api::PlayerExperience::from(
            mod_api::__private::built::PlayerExperience {
                level: 7,
                progress: 0.0,
                total: 0,
            },
        )));
        check("Backend", "original_player_experience", input(), expected);
        let mut expected = Writer::new();
        expected.put(&Some(vec![mod_api::ActiveEffect::new(
            mod_platform::__private::part_tests::effect_key("minecraft:speed"),
            200,
            1,
            false,
            true,
            true,
        )]));
        check("Backend", "original_player_effects", input(), expected);
        let mut expected = Writer::new();
        expected.put(&Some(
            Identifier::parse(if death {
                Dim::Nether.id()
            } else {
                Dim::End.id()
            })
            .unwrap(),
        ));
        check("Backend", "original_player_dimension", input(), expected);
        let mut args = Writer::new();
        args.put(&Holder::OriginalPlayer(0));
        args.put(&"clone");
        args.put(&"saved");
        let saved: horizon_core::nbt::Compound =
            [("value".into(), horizon_core::nbt::Tag::Int(31))].into();
        let mut expected = Writer::new();
        expected.put(&Ok::<_, mod_api::AttachmentError>(Some(saved)));
        check("AttachmentStore", "saved", args, expected);
        let mut args = input();
        args.put(&2i32);
        let mut expected = Writer::new();
        use super::super::api_values::TryIntoApi;
        let stack: Option<mod_api::ItemStack> = if death && !keep {
            None
        } else {
            Some(
                horizon_items::items::ItemStack::new("minecraft:diamond", 4)
                    .try_into_api()
                    .unwrap(),
            )
        };
        expected.put(&stack);
        check("Backend", "original_player_inventory_slot", args, expected);
        let stale =
            horizon_mod_wasm::fixture::import_id("Backend", "original_player_position").unwrap();
        // A production host write after all checks makes a trapped/undispatched guest observable.
        let player = host.player(event.player).unwrap();
        let mut marker = Writer::new();
        marker.put(&player);
        marker.put(&42i32);
        let setter =
            horizon_mod_wasm::fixture::import_id("BackendMut", "player_set_experience_levels")
                .unwrap();
        data.push_str(&format!(
            "(data (i32.const 24576) \"{}\")",
            escape(marker.bytes())
        ));
        let bytes = wat::parse_str(horizon_mod_wasm::fixture::by_name(&format!(r#"(module
          (@custom "dylink.0" "\01\04\00\00\00\00")
          (import "env" "memory" (memory 1))
          (import "env" "__indirect_function_table" (table 1 funcref))
          (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
          (import "horizon" "host_reply" (func (param i32)))
          (global $original (mut i64) (i64.const 0))
          (global $called (mut i32) (i32.const 0))
          (func (export "horizon_layout") (result i64) i64.const {layout})
          (func (export "horizon_in") (param i32) (result i32) i32.const 512)
          {data}
          (func (export "horizon_hook") (param i32) (result i64)
            global.get $called
            if
              i32.const 4096 global.get $original i64.store
              i32.const {stale} i32.const 4096 i32.const 8 i32.const 32768 i32.const 8192 call $host drop
              i32.const 32768 i32.load8_u if unreachable end
            else
              ;; UUID, length-prefixed name, was_death, then OriginalPlayerRef.
              i32.const 533 i32.const 528 i32.load i32.add i64.load global.set $original
              i32.const 1 global.set $called
              {body}
            end
            i32.const {setter} i32.const 24576 i32.const {len} i32.const 32768 i32.const 8192 call $host drop
            ;; Mutable event fields must be returned even when this listener leaves them unchanged.
            i32.const 28672 i32.const 512 i32.const 16 memory.copy
            i32.const 28688 i32.const 532 i32.const 528 i32.load i32.add i32.load8_u i32.store8
            i32.const 28689 global.get $original i64.store
            i64.const {reply}))"#, layout=*mod_api::wire::LAYOUT, len=marker.bytes().len(), reply=(28672u64 << 32) | 25), &[
                ("Backend", "original_player_position"),
                ("Backend", "original_player_health"),
                ("Backend", "original_player_experience"),
                ("Backend", "original_player_effects"),
                ("Backend", "original_player_dimension"),
                ("Backend", "original_player_inventory_slot"),
                ("AttachmentStore", "saved"),
                ("BackendMut", "player_set_experience_levels"),
            ])).unwrap();
        let module = horizon_mod_wasm::fixture::module(&bytes).unwrap();
        module.player_clone(host, event);
        assert_eq!(player.experience(host).unwrap().level, 42);
        module
    }

    struct Listener {
        death: bool,
        keep: bool,
        heard: Arc<Mutex<Vec<(OriginalPlayerRef, horizon_mod_wasm::WasmMod)>>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn player_clone(&self, host: &mut dyn ServerHost, event: &mut PlayerCloneEvent<'_>) {
            assert_eq!(event.was_death, self.death);
            let original = event.original;
            let holder = Holder::OriginalPlayer(original.fields());
            assert_eq!(original.uuid(host), Some(12));
            assert_eq!(original.position(host), Some([7.0, 64.0, -9.0]));
            let dimension = if self.death { Dim::Nether } else { Dim::End };
            assert_eq!(
                original.dimension(host).unwrap().to_string(),
                dimension.id()
            );
            assert_eq!(original.level(host).unwrap().dimension_id(), dimension.id());
            assert_eq!(VALUE.get(host, holder), Ok(73));
            const MISSING: AttachmentType<i32> = AttachmentType {
                name: "missing",
                ..VALUE
            };
            assert_eq!(original.attachment(host, &MISSING), Ok(0));
            assert_eq!(original.attachment(host, &VALUE), Ok(73));
            assert_eq!(original.attachment(host, &RETAINED), Ok(17));
            // The saved import reads the raw codec-backed entry, before native reads decode it.
            let wasm = wasm_reads(host, event, self.death, self.keep);
            assert_eq!(original.attachment(host, &SAVED), Ok(31));
            assert_eq!(
                SAVED.get(host, Holder::Player(event.player)),
                Ok(if self.death { 0 } else { 31 })
            );
            assert_eq!(
                VALUE.get(host, Holder::Player(event.player)),
                Ok(if self.death { 0 } else { 73 })
            );
            assert_eq!(RETAINED.get(host, Holder::Player(event.player)), Ok(17));
            assert_eq!(
                VALUE.set(host, holder, 100),
                Err(mod_api::AttachmentError::MissingHolder)
            );
            assert_eq!(
                VALUE.remove(host, holder),
                Err(mod_api::AttachmentError::MissingHolder)
            );
            if self.death && !self.keep {
                assert_eq!(original.inventory_slot(host, 2), None);
                assert_eq!(original.inventory_slot(host, 100), None);
                assert_eq!(original.inventory_slot(host, -106), None);
            } else {
                assert_eq!(original.inventory_slot(host, 2).unwrap().count(), 4);
                assert_eq!(original.inventory_slot(host, 100).unwrap().count(), 1);
                assert_eq!(original.inventory_slot(host, -106).unwrap().count(), 2);
            }
            assert_eq!(
                original.health(host),
                Some(if self.death { 0.0 } else { 13.0 })
            );
            assert_eq!(original.experience(host).unwrap().level, 7);
            let effects = original.effects(host).unwrap();
            assert_eq!(effects.len(), 1);
            assert_eq!(effects[0].duration, 200);
            assert_eq!(effects[0].amplifier, 1);
            let replacement = host.player(event.player).unwrap();
            let replacement_effects = mod_api::LivingRef::new(replacement.entity())
                .effects(host)
                .unwrap();
            assert_eq!(replacement_effects.len(), usize::from(!self.death));
            assert_eq!(original.inventory_slot(host, 99), None);
            // Replacement writes cannot alter the captured old values.
            VALUE.set(host, Holder::Player(event.player), 99).unwrap();
            assert_eq!(original.attachment(host, &VALUE), Ok(73));
            self.heard.lock().unwrap().push((original, wasm));
        }
    }

    fn clone_case(death: bool, keep: bool) {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "clone",
            Box::new(Listener {
                death,
                keep,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                let dimension = if death { Dim::Nether } else { Dim::End };
                game.enter_level(dimension).unwrap();
                game.ui.rules.set(
                    "keep_inventory",
                    horizon_client::gamerules::Value::Bool(keep),
                );
                let host = game.lan.as_mut().unwrap();
                host.hub.guests.update(0, |guest| {
                    guest.dimension = dimension.id().to_owned();
                    guest.position = [7.0, 64.0, -9.0];
                    guest.dead = death;
                    guest.health = if death { 0.0 } else { 13.0 };
                    guest.xp_level = 7;
                    guest
                        .effects
                        .list
                        .push(horizon_client::effects::EffectInstance::from_def(
                            horizon_client_core::potions::effect_by_index(
                                mod_api::__private::effect_index(
                                    mod_platform::__private::part_tests::effect_key(
                                        "minecraft:speed",
                                    ),
                                )
                                .unwrap(),
                            )
                            .unwrap(),
                            200,
                            1,
                        ));
                    // Respawn starts after death drops; keep-inventory leaves these intact.
                    if !death || keep {
                        guest.inventory[2] =
                            Some(horizon_items::items::ItemStack::new("minecraft:diamond", 4));
                        guest.armor[3] = Some(horizon_items::items::ItemStack::new(
                            "minecraft:diamond_boots",
                            1,
                        ));
                        guest.offhand =
                            Some(horizon_items::items::ItemStack::new("minecraft:apple", 2));
                    }
                });
                let attachments = host.hub.guests.attachments_mut(12).unwrap();
                RETAINED.set(attachments, Holder::Level, 17).unwrap();
                SAVED.set(attachments, Holder::Level, 31).unwrap();
                // Keep codec-backed entries undecoded, beside an unserialized binding.
                let saved = mod_api::Attachments::from_nbt(attachments.to_nbt());
                attachments.replace_contents(saved);
                VALUE.set(attachments, Holder::Level, 73).unwrap();
                if death {
                    game.guest_respawn(12);
                } else {
                    let play = host.play.entry(12).or_default();
                    play.won_game = true;
                    play.respawn_requested = true;
                    game.handle_server_portal(12);
                }
                let heard = heard.lock().unwrap();
                assert_eq!(heard.len(), 1);
                let (original, wasm) = &heard[0];
                let original = *original;
                let replacement = game.player(12).unwrap();
                assert!(game.player_set_experience_levels(replacement, 0));
                // Re-enter the same guest after dispatch; it retained the actual event token.
                let mut expired = PlayerCloneEvent {
                    player: 12,
                    name: "expired",
                    was_death: death,
                    original,
                };
                wasm.player_clone(game, &mut expired);
                assert_eq!(replacement.experience(game).unwrap().level, 42);
                assert_eq!(original.position(game), None);
                assert_eq!(original.inventory_slot(game, 2), None);
                assert_eq!(original.health(game), None);
                assert_eq!(original.experience(game), None);
                assert_eq!(original.effects(game), None);
                assert_eq!(
                    original.attachment(game, &VALUE),
                    Err(mod_api::AttachmentError::MissingHolder)
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn death_clone_retains_original_and_expires_after_dispatch() {
        crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
            clone_case(true, false);
            clone_case(true, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn end_return_clone_retains_end_state_and_expires_after_dispatch() {
        crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
            clone_case(false, true);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn unsubscribed_clone_does_not_capture() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_guests(|game| assert!(game.capture_original_player(12).is_none()));
        });
    }
}
