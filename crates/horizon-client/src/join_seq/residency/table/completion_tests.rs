use super::*;
use crate::join_seq::{ResidentList, ResidentSlot, residency::Kind};
use crate::test_log::RECORDS as DIAGNOSTICS;

#[derive(Clone)]
struct Stored {
    id: u32,
    join: JoinSeq,
}
crate::impl_joined!(Stored);

#[test]
fn abandoned_resident_wrapper_passes_are_expired_and_logged_even_in_release() {
    crate::test_log::install();
    eprintln!(
        "PASS_DIAGNOSTIC_ASSERTIONS_DISABLED:{}",
        !cfg!(debug_assertions)
    );
    for dimension in [DimensionKey::OVERWORLD, DimensionKey::NETHER] {
        for extraction in [false, true] {
            DIAGNOSTICS.with_borrow_mut(Vec::clear);
            let journal = Journal::default();
            let mut table = EntityTable::new(journal.clone());
            let owner = Owner {
                dimension,
                kind: Kind::Projectile,
            };
            if extraction {
                let mut slot = ResidentSlot::default();
                slot.bind_residency(&journal, owner, |entity: &Stored| (entity.id, entity.join));
                slot.replace(Stored {
                    id: 7,
                    join: JoinSeq::UNJOINED,
                });
                let held = slot.extract().unwrap();
                assert_eq!(
                    table.entity_lookup::<()>(&dimension, 7, |_| None),
                    EntityLookup::Taken
                );
                // Leave this scope without the required terminal retirement.
                assert_eq!(held.id, 7);
            } else {
                let mut list = ResidentList::default();
                list.bind_residency(&journal, owner, |entity: &Stored| (entity.id, entity.join));
                list.push(Stored {
                    id: 7,
                    join: JoinSeq::UNJOINED,
                });
                let outer = list.lift();
                let inner = list.lift();
                assert_eq!(
                    table.entity_lookup::<()>(&dimension, 7, |_| None),
                    EntityLookup::Taken
                );
                // Neither nonempty nor empty abandoned batches close a pass.
                drop(inner);
                drop(outer);
            }
            let expected = if extraction {
                UnbalancedPass {
                    extractions: 1,
                    lifts: 0,
                }
            } else {
                UnbalancedPass {
                    extractions: 0,
                    lifts: 2,
                }
            };
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| table.end_tick()));
            if cfg!(debug_assertions) {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), Err(expected));
            }
            DIAGNOSTICS.with_borrow(|records| {
                assert_eq!(
                    records.len(),
                    1,
                    "completion must emit one release-visible diagnostic"
                );
                assert_eq!(records[0].0, log::Level::Error);
                assert!(
                    records[0]
                        .1
                        .contains(&format!("extractions: {}", expected.extractions))
                );
                assert!(records[0].1.contains(&format!("lifts: {}", expected.lifts)));
            });
            assert!(table.pending.is_empty());
            assert!(table.extracted.is_empty());
            assert!(table.lifted.is_empty());
            let mut undrained = Vec::new();
            journal.drain_into(&mut undrained);
            assert!(undrained.is_empty());
            assert_eq!(
                table.entity_lookup::<()>(&dimension, 7, |_| None),
                EntityLookup::Missing
            );
            assert_eq!(table.end_tick(), Ok(()));
            DIAGNOSTICS.with_borrow(|records| assert_eq!(records.len(), 1));
        }
    }
}
