use horizon_mod_runtime::block_definitions::{BlockDefinition, DefinitionError, StateError};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    collections::BTreeMap,
    sync::Arc,
};

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}
struct CountingAllocator;
fn allocated() {
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}
// SAFETY: all operations delegate unchanged to System; counting is thread-local.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocated();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn fixture() -> BlockDefinition {
    let states: Vec<BTreeMap<_, _>> = [("north", "false"), ("south", "false"), ("south", "true")]
        .into_iter()
        .map(|(facing, lit)| {
            BTreeMap::from([
                ("facing".to_owned(), facing.to_owned()),
                ("lit".to_owned(), lit.to_owned()),
            ])
        })
        .collect();
    BlockDefinition::new(&states.iter().collect::<Vec<_>>(), 1).unwrap()
}

#[test]
fn definitions_preserve_ordinals_and_refuse_invalid_edits() {
    let table = fixture();
    assert_eq!(table.property(0, "facing"), Ok(Some("north")));
    assert_eq!(table.property(2, "lit"), Ok(Some("true")));
    assert_eq!(table.property(0, "missing"), Ok(None));
    assert_eq!(table.property(3, "lit"), Err(StateError::UnknownState));
    assert_eq!(table.of(&[]), Ok(1));
    assert_eq!(table.of(&[("lit", "true")]), Ok(2));
    assert_eq!(table.with(0, "facing", "south"), Ok(1));
    assert_eq!(
        table.with(2, "facing", "north"),
        Err(StateError::InvalidCombination)
    );
    assert_eq!(
        table.with(0, "missing", "x"),
        Err(StateError::UnknownProperty(0))
    );
    assert_eq!(
        table.with(0, "lit", "red"),
        Err(StateError::InvalidValue(0))
    );
    assert_eq!(
        table.of(&[("lit", "true"), ("lit", "false")]),
        Err(StateError::DuplicateProperty(1))
    );
    assert_eq!(table.property(2, "facing"), Ok(Some("south")));
}

#[test]
fn worker_property_reads_share_immutable_values_and_allocate_nothing() {
    let table = Arc::new(fixture());
    let worker = Arc::clone(&table);
    let (allocations, length) = std::thread::spawn(move || {
        ALLOCATIONS.with(|count| count.set(Some(0)));
        let mut length = 0;
        for ordinal in 0..1000 {
            let state = std::hint::black_box((ordinal % 3) as u16);
            length += worker
                .property(state, std::hint::black_box("facing"))
                .unwrap()
                .unwrap()
                .len();
            assert_eq!(worker.property(state, "missing"), Ok(None));
            assert_eq!(worker.of(std::hint::black_box(&[])), Ok(1));
        }
        (
            ALLOCATIONS.with(|count| count.replace(None).unwrap()),
            length,
        )
    })
    .join()
    .unwrap();
    assert_eq!(allocations, 0);
    assert_eq!(length, 5000);
    assert_eq!(table.property(2, "lit"), Ok(Some("true")));
}

#[test]
fn malformed_definitions_refuse_before_publication() {
    let empty = BTreeMap::new();
    let different = BTreeMap::from([("lit".to_owned(), "false".to_owned())]);
    assert!(matches!(
        BlockDefinition::new(&[], 0),
        Err(DefinitionError::Empty)
    ));
    assert!(matches!(
        BlockDefinition::new(&[&empty], 1),
        Err(DefinitionError::InvalidDefault)
    ));
    assert!(matches!(
        BlockDefinition::new(&[&empty, &different], 0),
        Err(DefinitionError::InconsistentProperties)
    ));
    assert!(matches!(
        BlockDefinition::new(&[&empty, &empty], 0),
        Err(DefinitionError::DuplicateState)
    ));
    assert!(matches!(
        BlockDefinition::new(&vec![&empty; 65_537], 0),
        Err(DefinitionError::TooManyStates)
    ));
}
