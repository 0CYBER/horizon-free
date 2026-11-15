//! Real generated host/guest prepared dispatch, using the existing loopback transport.
use super::prepared as slots;
use super::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
thread_local! {
    static REGISTER_PART: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
    static EXPECTED: RefCell<Option<mod_api::ItemStack>> = const { RefCell::new(None) };
    static ADDRESSES: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    static SLOTS: RefCell<std::collections::HashMap<u8, slots::Slots>> = RefCell::default();
    static COPIES: RefCell<(Vec<Vec<u8>>,Vec<usize>,usize)> = RefCell::default();
    static HELMET: std::cell::Cell<Option<mod_api::ItemKey>> = const { std::cell::Cell::new(None) };
}
pub(super) fn register(r: &mut dyn mod_api::Registrar) {
    HELMET.set(Some(r.item_key("minecraft:iron_helmet")));
    let part = || FeetOnly(Arc::new(AtomicUsize::new(0)), true);
    match REGISTER_PART.get() {
        1 => r.item_part(HELMET.get().unwrap(), part()),
        2 => r.item_part_tag("test:helmets", part()),
        _ => {}
    }
}
/// An item key the install registered, for tests that need a stack.
pub(super) fn helmet() -> mod_api::ItemKey {
    HELMET.get().unwrap()
}
pub(super) fn record(id: u32, bytes: &[u8]) {
    COPIES.with(|copies| {
        let mut copies = copies.borrow_mut();
        if id == slots::PREPARE {
            copies.0.push(bytes.to_vec());
        }
        if Some(id) == boundary::export_id("ItemPart", "can_equip") {
            copies.1.push(bytes.len());
        }
        if id == slots::RELEASE {
            copies.2 += 1;
        }
    });
}
impl WasmMod {
    pub(super) fn equip_instance(&self) -> usize {
        usize::from(self.group)
    }
    pub(super) fn prepare(&self, bytes: &[u8]) -> Option<(u32, u32)> {
        let token =
            SLOTS.with(|slots| slots.borrow_mut().entry(self.group).or_default().prepare())?;
        let echoed = self.call(
            CallClass::Hook,
            boundary::Frame::default(),
            slots::PREPARE,
            None,
            |w| {
                w.put(&token.0);
                w.put(&token.1);
                w.put_bytes(bytes);
            },
            |r| r.take::<(u32, u32)>(),
        );
        if echoed != Some(token) {
            self.release(token.0, token.1);
            return None;
        }
        Some(token)
    }
    pub(super) fn check(&self, slot: u32, generation: u32) -> bool {
        SLOTS.with(|slots| {
            slots
                .borrow()
                .get(&self.group)
                .is_some_and(|slots| slots.check(slot, generation))
        })
    }
    pub(super) fn release(&self, slot: u32, generation: u32) {
        if SLOTS.with(|slots| {
            slots
                .borrow_mut()
                .get_mut(&self.group)
                .is_some_and(|slots| slots.release(slot, generation))
        }) {
            self.call(
                CallClass::Hook,
                boundary::Frame::default(),
                slots::RELEASE,
                None,
                |w| {
                    w.put(&slot);
                    w.put(&generation);
                },
                |r| r.take::<()>(),
            );
        }
    }
}
struct World;
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World);
struct FeetOnly(Arc<AtomicUsize>, bool);
#[mod_api::horizon_part]
impl mod_api::ItemPart for FeetOnly {
    fn can_equip(&self, _: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
        self.0.fetch_add(1, Ordering::Relaxed);
        EXPECTED.with(|expected| assert_eq!(Some(event.stack), expected.borrow().as_ref()));
        if self.1 {
            ADDRESSES.with(|addresses| {
                addresses
                    .borrow_mut()
                    .push(event.stack as *const _ as usize)
            });
        }
        event.can_equip = event.slot == "feet";
    }
}
fn event<'a>(stack: &'a mod_api::ItemStack, slot: &'a str) -> mod_api::CanEquipEvent<'a> {
    mod_api::CanEquipEvent {
        stack,
        slot,
        entity: mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(1), 21)),
        kind: "minecraft:zombie",
        can_equip: true,
    }
}
fn proxy(count: Arc<AtomicUsize>, group: u8) -> WasmHandler {
    WasmHandler {
        index: guest::add_handler(vec![guest::view::<dyn mod_api::ItemPart>(Arc::new(
            FeetOnly(count, true),
        ))]),
        module: WasmMod {
            group,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    }
}
#[test]
fn prepared_can_equip_writes_stack_once_for_eight_asks() {
    use mod_api::ItemPart;
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let plain = mod_api::ItemStack::new(HELMET.get().unwrap(), 1).unwrap();
    let mut rich = plain.clone();
    rich.set_components(horizon_core::nbt::Compound::from([(
        horizon_core::nbt::JavaString::from("minecraft:custom_name"),
        horizon_core::nbt::Tag::String("C".repeat(4096).as_str().into()),
    )]))
    .unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let proxy = proxy(count.clone(), 0);
    let native = FeetOnly(Arc::new(AtomicUsize::new(0)), false);
    let mut ask_sizes = Vec::new();
    for stack in [&plain, &rich] {
        EXPECTED.with(|expected| *expected.borrow_mut() = Some(stack.clone()));
        ADDRESSES.with(|addresses| addresses.borrow_mut().clear());
        COPIES.with(|copies| *copies.borrow_mut() = Default::default());
        let mut encoded = Writer::new();
        encoded.put(stack);
        {
            let prepared = proxy
                .prepare_can_equip(stack, &["minecraft:zombie"])
                .unwrap();
            for _ in 0..2 {
                let actual = ["head", "chest", "legs", "feet"].map(|slot| {
                    let mut e = event(stack, slot);
                    proxy.ask_can_equip(&prepared, &World, &mut e);
                    e.can_equip
                });
                let native_answers = ["head", "chest", "legs", "feet"].map(|slot| {
                    let mut e = event(stack, slot);
                    native.can_equip(&World, &mut e);
                    e.can_equip
                });
                assert_eq!(actual, [false, false, false, true]);
                assert_eq!(actual, native_answers);
            }
        }
        ADDRESSES.with(|addresses| {
            let addresses = addresses.borrow();
            assert_eq!(addresses.len(), 8);
            assert!(addresses.iter().all(|address| *address == addresses[0]));
        });
        COPIES.with(|copies| {
            let copies = copies.borrow();
            assert_eq!(copies.0.len(), 1);
            assert!(copies.0[0][8..].starts_with(encoded.bytes()));
            assert_eq!(copies.1.len(), 8);
            assert_eq!(copies.2, 1);
            assert!(copies.1.iter().all(|n| *n == copies.1[0]));
            ask_sizes.push(copies.1[0]);
        });
    }
    assert_eq!(ask_sizes[0], ask_sizes[1]);
    assert_eq!(count.load(Ordering::Relaxed), 16);
}
#[test]
fn prepared_can_equip_refuses_stale_and_foreign_tokens_without_callback() {
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let stack = mod_api::ItemStack::new(HELMET.get().unwrap(), 1).unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let first = proxy(count.clone(), 0);
    let other = proxy(count.clone(), 1);
    let prepared = first
        .prepare_can_equip(&stack, &["minecraft:zombie"])
        .unwrap();
    let mut e = event(&stack, "feet");
    other.ask_can_equip(&prepared, &World, &mut e);
    assert!(!e.can_equip);
    assert_eq!(count.load(Ordering::Relaxed), 0);
    // Send raw stale requests through the generated guest dispatcher, bypassing host checks.
    let token = SLOTS.with(|slots| {
        let slots = slots.borrow();
        COPIES.with(|copies| {
            let copies = copies.borrow();
            let mut r = Reader::new(copies.0.last().unwrap());
            let slot: u32 = r.take().unwrap();
            let generation: u32 = r.take().unwrap();
            assert!(slots.get(&0).unwrap().check(slot, generation));
            (slot, generation)
        })
    });
    let ask = |generation| {
        first.module.call(
            CallClass::Hook,
            boundary::Frame::default(),
            boundary::export_id("ItemPart", "can_equip").unwrap(),
            Some(first.index),
            |w| {
                w.put(&token.0);
                w.put(&generation);
                w.put(&3u32);
                w.put(&event(&stack, "feet").entity);
                w.put(&0u32);
                w.put(&true);
            },
            |r| r.take::<()>(),
        )
    };
    assert!(ask(token.1 + 1).is_none());
    drop(prepared);
    assert!(ask(token.1).is_none());
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

impl WasmMod {
    pub(super) fn check_prepared_hooks(
        &self,
        hooks: &[Option<mod_api::HookId>],
    ) -> Result<(), String> {
        if REGISTER_PART.get() != 0
            && hooks
                .iter()
                .flatten()
                .any(|hook| matches!(hook, mod_api::HookId::CanEquip))
        {
            return Err("can_equip requires prepared wasm export prepare_can_equip; no fallback is available".into());
        }
        Ok(())
    }
}

#[test]
fn prepared_item_part_missing_exports_refuses_registration() {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            REGISTER_PART.set(0);
        }
    }
    let _reset = Reset;
    for mode in [1, 2] {
        REGISTER_PART.set(mode);
        let mods = [mod_api::ModEntry::new(
            "wire_test",
            Box::new(WasmMod {
                group: 0,
                subscribed: &[],
                listeners: &[],
                corrupt: false,
            }),
        )];
        let error = mod_api::__private::install_parts(
            &mods,
            &mod_api::__private::part_tests::rule_schema(),
            |_| false,
        )
        .unwrap_err();
        assert!(format!("{error:?}").contains("prepare_can_equip"));
    }
}

struct PreparedReads;
#[mod_api::horizon_part]
impl mod_api::ItemPart for PreparedReads {
    fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
        let entity = event.entity.entity();
        assert_eq!(entity.kind(host), None);
        assert_eq!(entity.position(host), Some([1.0, 64.0, 2.0]));
        assert_eq!(event.entity.health(host), Some(17.0));
        assert_eq!(event.entity.max_health(host), Some(20.0));
        assert_eq!(event.entity.alive(host), Some(true));
        let held = event.entity.hand_item(host, mod_api::Hand::Main).unwrap();
        assert_eq!(held.count(), 3);
        assert!(held.components().is_some());
        assert!(
            event
                .entity
                .equipment(host, mod_api::EquipmentSlot::Head)
                .is_none()
        );
        event.can_equip = event.slot == "feet";
    }
}
#[test]
fn prepared_target_reads_need_no_host_round_trip_or_payload_conversion() {
    install_counter();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut stack = mod_api::ItemStack::new(HELMET.get().unwrap(), 3).unwrap();
    stack
        .set_components(horizon_core::nbt::Compound::from([(
            horizon_core::nbt::JavaString::from("minecraft:custom_name"),
            horizon_core::nbt::Tag::String("C".repeat(4096).as_str().into()),
        )]))
        .unwrap();
    let entity = event(&stack, "head").entity.entity();
    let mut equipment: [Option<mod_api::ItemStack>; 8] = Default::default();
    equipment[0] = Some(stack.clone());
    let row = mod_api::EquipReadRow {
        position: Some([1.0, 64.0, 2.0]),
        health: Some(17.0),
        max_health: Some(20.0),
        alive: Some(true),
        living: true,
        equipment,
        ..mod_api::EquipReadRow::new(entity)
    };
    let proxy = WasmHandler {
        index: guest::add_handler(vec![guest::view::<dyn mod_api::ItemPart>(Arc::new(
            PreparedReads,
        ))]),
        module: WasmMod {
            group: 7,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    let prepared = proxy
        .prepare_can_equip_with_reads(&stack, &["minecraft:zombie"], &[row])
        .unwrap();
    for _ in 0..2 {
        let answers = ["head", "chest", "legs", "feet"].map(|slot| {
            let mut event = event(&stack, slot);
            proxy.ask_can_equip(&prepared, &World, &mut event);
            event.can_equip
        });
        assert_eq!(answers, [false, false, false, true]);
    }
}

pub(super) struct PreparedEquipHooks;
impl PreparedEquipHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        _cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
