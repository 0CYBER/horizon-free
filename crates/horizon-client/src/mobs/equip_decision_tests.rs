use super::*;
use std::cell::Cell;
fn helmet() -> ItemStack {
    ItemStack::new("minecraft:iron_helmet", 3)
}
#[test]
fn every_source_mutation_refuses_the_old_preparation_without_calling() {
    for mutation in 0..5 {
        let mut source = Some(helmet());
        let mut decision = EquipDecision::new(42, &mut source);
        let stamp = decision.stamp().unwrap();
        match mutation {
            0 => *decision.count_mut().unwrap() = 2,
            1 => {
                decision.data_mut().unwrap().custom_name =
                    Some(horizon_core::nbt::Tag::String("Crown".into()))
            }
            2 => {
                decision.replace(Some(helmet()));
            }
            3 => {
                decision.remove();
            }
            4 => {
                *decision.count_mut().unwrap() = 2;
                *decision.count_mut().unwrap() = 3;
                assert_eq!(decision.stack(), Some(&helmet()));
            }
            _ => unreachable!(),
        }
        let called = Cell::new(0);
        assert!(!decision.ask(stamp, |_| {
            called.set(called.get() + 1);
            true
        }));
        assert_eq!(called.get(), 0);
    }
}
#[test]
fn taking_a_mutable_borrow_invalidates_before_any_write() {
    let mut source = Some(helmet());
    let mut decision = EquipDecision::new(42, &mut source);
    let stamp = decision.stamp().unwrap();
    let called = Cell::new(0);
    assert!(decision.ask(stamp, |stack| {
        assert_eq!(stack.count, 3);
        called.set(called.get() + 1);
        true
    }));
    let _borrow = decision.count_mut().unwrap();
    assert!(!decision.ask(stamp, |_| {
        called.set(called.get() + 1);
        true
    }));
    assert_eq!(called.get(), 1);
    assert!(!decision.matches(stamp));
    assert_eq!(decision.stack().unwrap().count, 3);
}
#[test]
fn stale_menu_or_entity_preconditions_refuse_apply() {
    let mut source = Some(helmet());
    let mut decision = EquipDecision::new(42, &mut source);
    let entity = EquipEntityStamp {
        world: 9,
        id: 21,
        generation: 7,
        revision: 3,
    };
    let answer = decision
        .answer(decision.stamp().unwrap(), 5, entity, true)
        .unwrap();
    let called = Cell::new(0);
    for (menu, current) in [
        (6, Some(entity)),
        (5, None),
        (
            5,
            Some(EquipEntityStamp {
                world: 10,
                ..entity
            }),
        ),
        (
            5,
            Some(EquipEntityStamp {
                generation: 8,
                ..entity
            }),
        ),
        (
            5,
            Some(EquipEntityStamp {
                revision: 4,
                ..entity
            }),
        ),
        (5, Some(EquipEntityStamp { id: 9999, ..entity })),
    ] {
        assert_eq!(
            decision.apply(answer, menu, current, |_| called.set(called.get() + 1)),
            None
        );
    }
    assert_eq!(called.get(), 0);
    assert_eq!(
        decision.apply(answer, 5, Some(entity), |source| {
            *source.count_mut().unwrap() = 2;
            9
        }),
        Some(9)
    );
    assert_eq!(decision.apply(answer, 5, Some(entity), |_| 0), None);
}
#[test]
fn replacement_and_removal_change_identity_and_overflow_retires_scope() {
    let mut source = Some(helmet());
    let mut decision = EquipDecision::new(42, &mut source);
    let first = decision.stamp().unwrap();
    *decision.count_mut().unwrap() = 2;
    assert_eq!(decision.stamp().unwrap().identity, first.identity);
    decision.replace(Some(helmet()));
    assert_ne!(decision.stamp().unwrap().identity, first.identity);
    let replaced = decision.stamp().unwrap();
    decision.remove();
    assert_ne!(decision.stamp.identity, replaced.identity);
    decision.replace(Some(helmet()));
    decision.stamp.revision = u64::MAX;
    let last = decision.stamp().unwrap();
    let _borrow = decision.count_mut();
    assert!(decision.stamp().is_none());
    assert_eq!(
        crate::mobs::equip::source_stamp_at(decision.stack().unwrap() as *const _ as usize),
        Some(None)
    );
    assert!(!decision.ask(last, |_| true));
}
#[test]
fn stamps_from_another_location_or_scope_refuse() {
    let mut left = Some(helmet());
    let mut right = Some(helmet());
    let first = EquipDecision::new(42, &mut left);
    let second = EquipDecision::new(42, &mut right);
    assert!(!second.matches(first.stamp().unwrap()));
    let mut token = second.stamp().unwrap();
    token.location = 43;
    assert!(!second.matches(token));
}
