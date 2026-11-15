use super::*;
use crate::join_seq::residency::EntitySubtype;

#[test]
fn cold_identity_details_survive_real_loan_and_stale_removal_but_not_absence() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    let owner = Owner {
        dimension: DimensionKey::OVERWORLD,
        kind: super::super::Kind::Mob,
    };
    let old = Identity::new(73, JoinSeq::next())
        .unwrap()
        .with_known_uuid(Some([1, 2, 3, 4]))
        .with_subtype(EntitySubtype::Merchant);
    journal.push(Event {
        owner,
        change: Change::Install(old),
    });
    let candidates: Vec<_> = table.admitted_metadata().collect();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].0, 73);
    assert_eq!(candidates[0].2.known_uuid, Some([1, 2, 3, 4]));
    assert_eq!(candidates[0].2.subtype, EntitySubtype::Merchant);
    journal.push(Event {
        owner,
        change: Change::Extract(old),
    });
    let (at, state, details) = table.server_admitted_identity::<()>(73, |_| None).unwrap();
    assert_eq!(state, EntityLookup::Taken);
    assert_eq!(at, candidates[0].1);
    assert_eq!(details, candidates[0].2);
    assert_eq!(table.admitted_metadata().count(), 1);

    journal.push(Event {
        owner,
        change: Change::Remove(old),
    });
    let new = Identity::new(73, JoinSeq::next())
        .unwrap()
        .with_known_uuid(Some([5, 6, 7, 8]));
    journal.push(Event {
        owner,
        change: Change::Install(new),
    });
    journal.push(Event {
        owner,
        change: Change::Remove(old),
    });
    let (new_at, state, details) = table
        .server_admitted_identity(73, |_| Some(SourceEntity { id: 73, value: 19 }))
        .unwrap();
    assert_eq!(
        state,
        EntityLookup::Found {
            location: new_at,
            value: 19
        }
    );
    assert_ne!(new_at, at);
    assert_eq!(details.known_uuid, Some([5, 6, 7, 8]));
    assert_eq!(details.subtype, EntitySubtype::Ordinary);
    assert!(table.server_admitted_identity::<()>(73, |_| None).is_none());
    assert_eq!(table.admitted_metadata().count(), 0);
}

#[test]
fn candidate_iteration_does_not_resolve_or_invent_physical_membership() {
    let journal = Journal::default();
    let mut table = EntityTable::new(journal.clone());
    let owner = Owner {
        dimension: DimensionKey::NETHER,
        kind: super::super::Kind::Other,
    };
    let identity = Identity::new(91, JoinSeq::next())
        .unwrap()
        .with_subtype(EntitySubtype::TextDisplay);
    journal.push(Event {
        owner,
        change: Change::Install(identity),
    });
    let candidates: Vec<_> = table.admitted_metadata().collect();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].2.subtype, EntitySubtype::TextDisplay);
    // A candidate remains subject to the sole exact physical-source resolver.
    // The wrong actual ID is not accepted just because metadata was admitted.
    assert!(
        table
            .server_admitted_identity(91, |_| Some(SourceEntity { id: 92, value: () }))
            .is_none()
    );
    assert_eq!(table.admitted_metadata().count(), 0);
}
