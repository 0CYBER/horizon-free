use super::*;
use std::sync::{Arc, Mutex};

// Native mod registries are process-global. Each parallel test owns a fresh process
// and temporary working directory, while retaining the normal test runner parallelism.
pub(super) fn isolated() -> bool {
    const FLAG: &str = "HORIZON_DEATH_WEAPON_CHILD";
    if std::env::var_os(FLAG).is_some() {
        return false;
    }
    let scratch = tempfile::tempdir().unwrap();
    let resources = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap())
        .canonicalize()
        .unwrap();
    let thread = std::thread::current();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            thread.name().unwrap(),
            "--nocapture",
        ])
        .current_dir(scratch.path())
        .env(FLAG, "1")
        .env("HORIZON_TEST_RESOURCES", resources)
        .env("HORIZON_NO_AUDIO", "1")
        .env("HORIZON_LAUNCHER_NO_SELF_UPDATE", "1");
    for key in [
        "HORIZON_HOME",
        "HORIZON_LAUNCHER_HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
    ] {
        command.env(key, scratch.path().join(key));
    }
    assert!(command.status().unwrap().success());
    true
}

pub(super) struct WeaponProbe {
    pub(super) expected: Option<mod_api::ItemStack>,
    pub(super) swap: Option<mod_api::ItemStack>,
    pub(super) seen: Arc<Mutex<usize>>,
}
#[mod_api::__subscriptions]
impl mod_api::Hooks for WeaponProbe {
    fn loot_drops(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LootDropsEvent<'_>,
    ) {
        assert!(event.context.tool.is_none());
        assert_eq!(event.context.weapon_item(host), self.expected);
        if let Some(expected) = &self.expected {
            wasm_weapon_dispatch(host, event, expected);
        }
        if let Some(axe) = &self.swap {
            let source = event.context.damage_source.unwrap();
            let living = host
                .living(host.entity(source.causing as u32).unwrap())
                .unwrap();
            assert!(
                living
                    .set_hand_item(host, mod_api::Hand::Main, Some(axe.clone()))
                    .unwrap()
            );
            assert_eq!(event.context.weapon_item(host), Some(axe.clone()));
            wasm_weapon_dispatch(host, event, axe);
        }
        *self.seen.lock().unwrap() += 1;
    }
}

fn weapon_case(item: &str, enchantment: &str, level: u32, projectile: bool, swap: bool) {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        game.save = Arc::new(horizon_client::save::SaveDir::new(
            scratch.path().join("save"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        game.build_registries();
        game.next_entity = 1000;
        let loot = horizon_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        let ty = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        let mut weapon = ItemStack::new(item, 1);
        weapon.enchant(enchantment, level);
        if item == "minecraft:bow" {
            weapon
                .data_mut()
                .set_loot_name("minecraft:custom_name", &serde_json::json!("Longbow"));
        }
        let expected = weapon.try_into_api().unwrap();
        let patch = expected.components().unwrap().as_nbt();
        assert_eq!(
            patch.get(&horizon_core::nbt::JavaString::from(
                "minecraft:enchantments"
            )),
            Some(&mod_api::Tag::Compound(
                [(enchantment.into(), mod_api::Tag::Int(level as i32))].into()
            ))
        );
        if item == "minecraft:bow" {
            assert_eq!(
                patch.get(&horizon_core::nbt::JavaString::from(
                    "minecraft:custom_name"
                )),
                Some(&mod_api::Tag::String("Longbow".into()))
            );
        }

        let axe = ItemStack::new("minecraft:diamond_axe", 1)
            .try_into_api()
            .unwrap();
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "weapon_probe",
            Box::new(WeaponProbe {
                expected: Some(expected),
                swap: swap.then_some(axe),
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let attacker_kind = match item {
                "minecraft:bow" => "minecraft:skeleton",
                "minecraft:crossbow" => "minecraft:pillager",
                "minecraft:trident" => "minecraft:drowned",
                _ => "minecraft:zombie",
            };
            let attacker_type = game.res.mob_types().get(attacker_kind).unwrap().clone();
            let mut attacker = Mob::new(41, attacker_type, Vec3::ZERO, 0.0);
            attacker.extra.set_main_hand_stack(Some(weapon.clone()));
            game.mobs.push(attacker);
            let mut victim = Mob::new(70, ty, Vec3::new(2.0, 64.0, 0.0), 0.0);
            let mut damage = Damage::of(if projectile {
                DamageKind::Arrow
            } else {
                DamageKind::MobAttack
            });
            damage.attacker = Some(Who::Mob(41));
            victim.health = 0.0;
            victim.last_damage = Some((damage, 0));
            if projectile {
                let trident = item == "minecraft:trident";
                let kind = if trident {
                    ProjectileKind::Trident
                } else {
                    ProjectileKind::Arrow {
                        base_damage: 2.0,
                        critical: false,
                        effect: None,
                    }
                };
                let mut shot = Projectile::new(kind, Vec3::ZERO, Some(Who::Mob(41)));
                shot.id = 912;
                let arrow = shot.arrow.as_mut().unwrap();
                if trident {
                    arrow.pickup_item = weapon;
                } else {
                    arrow.fired_from_weapon = Some(weapon);
                }
                game.mobs
                    .iter_mut()
                    .find(|m| m.id == 41)
                    .unwrap()
                    .extra
                    .set_main_hand_stack(Some(ItemStack::new("minecraft:stick", 1)));
                // Commit through the same accepted-damage path as real projectile impacts.
                victim.health = 20.0;
                victim.last_damage = None;
                game.mobs.push(victim);
                damage.kind = game.projectile_damage_kind(&shot);
                assert!(game.damage_projectile_with_weapon(&shot, 70, 100.0, damage, None));
                victim = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
                drop(shot);
            }
            game.mob_died(&victim);
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_a_longbow_power_ii_survives_hand_change() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        weapon_case("minecraft:bow", "minecraft:power", 2, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_b_crossbow_piercing_i() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        weapon_case("minecraft:crossbow", "minecraft:piercing", 1, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_c_trident_loyalty_ii() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        weapon_case("minecraft:trident", "minecraft:loyalty", 2, true, false);
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_d_sharpness_v_sword_reads_live_axe() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        weapon_case(
            "minecraft:diamond_sword",
            "minecraft:sharpness",
            5,
            false,
            true,
        );
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_e_fall_has_no_weapon() {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        game.save = Arc::new(horizon_client::save::SaveDir::new(
            scratch.path().join("save"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        game.build_registries();
        let loot = horizon_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "fall_weapon_probe",
            Box::new(WeaponProbe {
                expected: None,
                swap: None,
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let ty = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut victim = Mob::new(70, ty, Vec3::ZERO, 0.0);
            victim.health = 0.0;
            victim.last_damage = Some((Damage::fall(), 0));
            game.mob_died(&victim);
        });
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}

// The producer calls WeaponProbe with its real context. Dispatch that same context to
// a real guest, checking either the recorded stack on the wire or the live backend.
fn wasm_weapon_dispatch(
    host: &mut dyn mod_api::ServerHost,
    event: &mut mod_api::LootDropsEvent<'_>,
    expected: &mod_api::ItemStack,
) {
    use mod_api::{Hooks, wire::Writer};
    let mut request = Writer::new();
    let mut answer = Writer::new();
    let recorded = event.context.weapon.is_some();
    if !recorded {
        let source = event.context.damage_source.unwrap();
        let entity = host.entity(source.direct as u32).unwrap();
        let living = host.living(entity).unwrap();
        request.put(&living);
        answer.put(&Some(expected.clone()));
    } else {
        answer.put(expected);
    }
    let escape = |bytes: &[u8]| {
        bytes
            .iter()
            .map(|b| format!("\\{b:02x}"))
            .collect::<String>()
    };
    let backend = horizon_mod_wasm::fixture::import_id("Backend", "living_weapon_item").unwrap();
    let check = if recorded {
        // Borrowed stacks are embedded in the hook frame. Find the complete encoded
        // stack, including count, damage and components, not merely its item ID.
        "i32.const 512 local.set $start global.get $length i32.const 512 i32.add local.set $end"
            .to_owned()
    } else {
        format!(
            r#"
          i32.const {backend} i32.const 4096 i32.const {request_len}
          i32.const 16384 i32.const 8192 call $host local.set $length
          local.get $length i32.const {answer_len} i32.ne if unreachable end
          i32.const 16384 local.set $start
          i32.const 16384 local.get $length i32.add local.set $end
        "#,
            request_len = request.bytes().len(),
            answer_len = answer.bytes().len()
        )
    };
    let bytes = wat::parse_str(format!(
        r#"(module
      (@custom "dylink.0" "\01\04\00\00\00\00")
      (import "env" "memory" (memory 1))
      (import "env" "__indirect_function_table" (table 1 funcref))
      (import "horizon" "host" (func $host (param i32 i32 i32 i32 i32) (result i32)))
      (import "horizon" "host_reply" (func (param i32)))
      (func (export "horizon_layout") (result i64) i64.const {layout})
      (global $length (mut i32) (i32.const 0))
      (data (i32.const 4096) "{request}")
      (data (i32.const 32768) "{answer}")
      (func (export "horizon_in") (param i32) (result i32)
        local.get 0 global.set $length i32.const 512)
      (func (export "horizon_hook") (param i32) (result i64)
        (local $start i32) (local $end i32) (local $i i32) (local $length i32)
        {check}
        (block $found (loop $search
          local.get $start i32.const {answer_len} i32.add local.get $end i32.gt_u if unreachable end
          i32.const 0 local.set $i
          (block $different (loop $compare
            local.get $i i32.const {answer_len} i32.eq br_if $found
            local.get $start local.get $i i32.add i32.load8_u
            i32.const 32768 local.get $i i32.add i32.load8_u i32.ne br_if $different
            local.get $i i32.const 1 i32.add local.set $i br $compare))
          local.get $start i32.const 1 i32.add local.set $start br $search))
        ;; Successful assertion returns an empty drops vector (u32 length zero).
        i32.const 49152 i32.const 0 i32.store
        i64.const 211106232532996)
    )"#,
        layout = *mod_api::wire::LAYOUT,
        request = escape(request.bytes()),
        answer = escape(answer.bytes()),
        answer_len = answer.bytes().len()
    ))
    .unwrap();
    let module = horizon_mod_wasm::fixture::module(&bytes).unwrap();
    let mut sentinel = expected.clone();
    sentinel
        .set_count(if expected.count() == 1 { 2 } else { 1 })
        .unwrap();
    let mut drops = vec![sentinel];
    module.loot_drops(
        host,
        &mut mod_api::LootDropsEvent {
            context: event.context,
            drops: &mut drops,
        },
    );
    assert!(
        drops.is_empty(),
        "guest must assert the producer weapon and return successfully"
    );
}
