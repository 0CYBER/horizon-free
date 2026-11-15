use super::*;
use mod_api::ItemPart;
struct ReadHost;
impl mod_api::Host for ReadHost {}
mod_api::impl_host_areas!(ReadHost);
#[test]
#[ignore = "daily"]
fn equip_scope_copies_once_per_instance_for_eight_candidates_and_two_parts() {
    crate::test_stack::on_guest_stack(|| {
        let ask_reply = (1024u64 << 32) | 13;
        let exports = format!(
            r#"
            (func (export "prepare_can_equip") (result i64) i64.const 2199023255560)
            (func (export "ask_can_equip") (result i64)
                (memory.copy (i32.const 1024) (i32.const 532) (i32.const 12))
                (i32.store8 (i32.const 1036) (i32.eq (i32.load (i32.const 528)) (i32.const 3)))
                i64.const {ask_reply})
            (func (export "release_can_equip") (result i64) i64.const 0)
        "#
        );
        let bytes = super::runtime_tests::guest_with_exports(
            *mod_api::wire::LAYOUT,
            "unreachable",
            &exports,
        );
        let engine = engine().unwrap();
        let make = || {
            WasmMod::instantiate(
                &engine,
                DEFAULT_GROUP_MEMORY_MIB,
                &[Member {
                    name: "equip_scope",
                    module: &bytes,
                }],
                None,
            )
            .unwrap()
            .remove(0)
        };
        let first = make();
        let second = make();
        let parts = [
            WasmHandler::new(&first, Id(1)),
            WasmHandler::new(&first, Id(2)),
            WasmHandler::new(&second, Id(1)),
        ];
        let item = mod_api::__private::parts_installation::item_key(0);
        let small = mod_api::ItemStack::new(item, 3).unwrap();
        let mut large = small.clone();
        large
            .set_components(horizon_core::nbt::Compound::from([(
                horizon_core::nbt::JavaString::from("minecraft:custom_name"),
                horizon_core::nbt::Tag::String("C".repeat(4096).as_str().into()),
            )]))
            .unwrap();
        let mut traversal_bytes = Vec::new();
        for stack in [&small, &large] {
            let (leases, setup) = crate::copy_probe::measure(|| {
                parts
                    .iter()
                    .filter_map(|part| {
                        part.__prepare_equip(stack, &["minecraft:zombie"], &[])
                            .unwrap()
                    })
                    .collect::<Vec<_>>()
            });
            assert_eq!(
                leases.len(),
                2,
                "two parts of one instance share its preparation"
            );
            let mut payload = Writer::new();
            payload.put(stack);
            payload.put(&vec!["minecraft:zombie".to_owned()]);
            payload.put(&Vec::<mod_api::EquipReadRow>::new());
            assert_eq!(setup.to_guest_bytes, 2 * (8 + payload.bytes().len()));
            let (_, counts) = crate::copy_probe::measure(|| {
                for n in 0..8 {
                    let slot = ["head", "chest", "legs", "feet"][n % 4];
                    for index in [n % 2, 2] {
                        let mut event = mod_api::CanEquipEvent {
                            stack,
                            slot,
                            entity: mod_api::LivingRef::new(mod_api::EntityRef::new(
                                mod_api::WorldEpoch::new(1),
                                21,
                            )),
                            kind: "minecraft:zombie",
                            can_equip: false,
                        };
                        parts[index].can_equip(&ReadHost, &mut event);
                        assert_eq!(event.can_equip, slot == "feet");
                    }
                }
            });
            assert_eq!(counts.to_guest_bytes, 2 * 8 * 37);
            assert_eq!(counts.from_guest_bytes, 2 * 8 * 13);
            traversal_bytes.push(counts.to_guest_bytes);
            drop(leases);
        }
        assert_eq!(traversal_bytes[0], traversal_bytes[1]);
    });
}
