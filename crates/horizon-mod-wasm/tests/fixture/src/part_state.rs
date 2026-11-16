//! Block entity Part state (arismoko/horizon#781), shared by the native and the compiled wasm run of the game
//! test.
use mod_api::*;
use std::sync::Arc;

pub static COUNT: AttachmentType<i32> = AttachmentType {
    namespace: "wasm_fixture",
    name: "part_state_count",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// What modes 1 and 3 set once their checks passed, so the game sees they ran to the end.
pub static MARK: AttachmentType<i32> = AttachmentType {
    namespace: "wasm_fixture",
    name: "part_state_mark",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// A type whose Wire form holds a resource handle: refused on every target.
type Held = Option<Arc<dyn BlockPart>>;
static HELD: AttachmentType<Held> = AttachmentType {
    namespace: "wasm_fixture",
    name: "part_state_held",
    default: || None,
    codec: None,
    copy_on_death: false,
    sync: false,
};

/// Each tick does what its block entity's first data slot says:
/// - 0: an aborted edit, a refused recursive edit, then a committed count;
/// - 1: an edit of an undecodable saved value is refused before its closure runs, then `MARK`
///   is set to 1;
/// - 2: the closure stores 99 and then traps (wasm only: natively the panic unwinds out of
///   the tick);
/// - 3: the declaration of a state holding a resource handle is refused, as compiled here,
///   then `MARK` is set to 3.
pub struct Counter;
#[mod_api::horizon_part]
impl BlockPart for Counter {}
#[mod_api::horizon_part]
impl BlockEntityPart for Counter {
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        let level = host.level(host.dimension()).unwrap();
        let entity = host.block_entity_identity(level, event.pos).unwrap();
        match event.entity.data[0] {
            1 => {
                let refused = Counter::edit_state(host, entity, |_, _| panic!("ran"));
                assert_eq!(
                    refused,
                    Err(PartStateError::Attachment {
                        namespace: "wasm_fixture",
                        name: "part_state_count",
                        error: AttachmentError::Decode,
                    })
                );
                assert_eq!(edit_part_state(host, entity, &MARK, |_, m| *m = 1), Ok(()));
            }
            2 => {
                let _ = Counter::edit_state(host, entity, |_, n| {
                    *n = 99;
                    panic!("the closure fails");
                });
            }
            3 => {
                for wasm in [false, true] {
                    assert_eq!(
                        PartStateDeclaration::of(&HELD).check(wasm),
                        Err(PartStateRefusal::ResourceHandle)
                    );
                }
                assert_eq!(edit_part_state(host, entity, &MARK, |_, m| *m = 3), Ok(()));
            }
            _ => {
                let aborted: Result<(), ()> = host.transaction(|host, _| {
                    assert_eq!(Counter::edit_state(host, entity, |_, n| *n = 100), Ok(()));
                    Err(())
                });
                assert_eq!(aborted, Err(()));
                let recursive = Counter::edit_state(host, entity, |host, n| {
                    *n += 1;
                    Counter::edit_state(host, entity, |_, n| *n = 50)
                });
                assert_eq!(recursive, Ok(Err(PartStateError::Reentrant)));
            }
        }
    }
}
impl BlockEntityPartState for Counter {
    type Data = i32;
    const DATA: &'static AttachmentType<i32> = &COUNT;
}

pub fn register(registrar: &mut dyn Registrar) {
    let kind = registrar
        .block_entity_type("part_state")
        .blocks(&["wasm_fixture:part_state"])
        .build();
    registrar
        .block_entity_part_with_state(&kind, Counter)
        .unwrap();
}

/// The block the state's block entity stands in, so a save keeps it.
pub fn blocks(event: &mut BlocksEvent<'_>) {
    event
        .blocks
        .push(Block::new("wasm_fixture:part_state", "minecraft:barrel").with_item(false));
}
