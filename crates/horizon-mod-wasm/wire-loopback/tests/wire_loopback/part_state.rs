//! Block entity Part state (arismoko/horizon#781) through the generated transport: a guest Part's tick finds its
//! block entity and edits its state in transactions, every call crossing.
use super::*;
use mod_api::{
    AttachmentType, BlockEntityPart, BlockEntityPartState, BlockEntityRef, BlockEntityTickEvent,
    HostTransaction, LevelRef, ServerHost,
};
use std::cell::RefCell;
use std::collections::HashMap;

static TICKS: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "ticks",
    default: || 0,
    codec: Some(mod_api::AttachmentCodec {
        encode: |n| Ok([("n".into(), mod_api::Tag::Int(*n))].into()),
        decode: |saved| match saved.get(&"n".into()) {
            Some(mod_api::Tag::Int(n)) => Ok(*n),
            _ => Err(mod_api::AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

type Key = (mod_api::Holder, String, String);
type Stored = (mod_api::AttachmentValue, Option<mod_api::Compound>);

/// One level with one block entity, an attachment map and a journal of whole-map copies.
#[derive(Default)]
struct Store {
    values: HashMap<Key, Stored>,
    journal: Vec<HashMap<Key, Stored>>,
    stores: usize,
}
impl mod_api::AttachmentStore for Store {
    fn holds(
        &mut self,
        holder: mod_api::Holder,
        namespace: &str,
        name: &str,
    ) -> Result<bool, mod_api::AttachmentError> {
        Ok(self
            .values
            .contains_key(&(holder, namespace.into(), name.into())))
    }
    fn decoded(
        &mut self,
        holder: mod_api::Holder,
        namespace: &str,
        name: &str,
    ) -> Result<Option<mod_api::AttachmentValue>, mod_api::AttachmentError> {
        Ok(self
            .values
            .get(&(holder, namespace.into(), name.into()))
            .map(|(value, _)| value.clone()))
    }
    fn saved(
        &mut self,
        _: mod_api::Holder,
        _: &str,
        _: &str,
    ) -> Result<Option<mod_api::Compound>, mod_api::AttachmentError> {
        Ok(None)
    }
    fn store(
        &mut self,
        holder: mod_api::Holder,
        namespace: &str,
        name: &str,
        value: mod_api::AttachmentValue,
        form: mod_api::AttachmentForm,
    ) -> Result<(), mod_api::AttachmentError> {
        self.stores += 1;
        self.values.insert(
            (holder, namespace.into(), name.into()),
            (value, form.encoded),
        );
        Ok(())
    }
    fn discard(
        &mut self,
        holder: mod_api::Holder,
        namespace: &str,
        name: &str,
        _: bool,
    ) -> Result<(), mod_api::AttachmentError> {
        self.values.remove(&(holder, namespace.into(), name.into()));
        Ok(())
    }
    fn queue_sync(
        &mut self,
        _: mod_api::Holder,
        _: &str,
        _: &str,
    ) -> Result<(), mod_api::AttachmentError> {
        Ok(())
    }
}

struct World {
    level: LevelRef,
    entity: BlockEntityRef,
    store: Store,
    tokens: u64,
}
impl mod_api::Host for World {}
mod_api::impl_host_areas!(World, except(config, entities, block_entities, mod_state));
impl mod_api::prelude::ConfigHost for World {
    fn dimension(&self) -> &'static str {
        "minecraft:overworld"
    }
}
impl mod_api::prelude::EntitiesHost for World {
    fn level(&self, dimension: &str) -> Option<LevelRef> {
        (dimension == "minecraft:overworld").then_some(self.level)
    }
}
impl mod_api::prelude::BlockEntitiesHost for World {
    fn block_entity_identity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        (level == self.level && pos == [1, 64, 1]).then_some(self.entity)
    }
}
impl mod_api::prelude::ModStateHost for World {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        &mut self.store
    }
}
impl ServerHost for World {}
mod_api::impl_server_host_areas!(World, except(mod_state));
impl mod_api::prelude::ModStateServerHost for World {
    fn transaction_open(&mut self) -> mod_api::TransactionSeal {
        self.store.journal.push(self.store.values.clone());
        self.tokens += 1;
        mod_api::__private::transaction_seal(self.tokens, self.tokens)
    }
    fn transaction_close(&mut self, _: &mod_api::TransactionSeal, commit: bool) {
        let kept = self.store.journal.pop().unwrap();
        if !commit {
            self.store.values = kept;
        }
    }
}

thread_local! {
    /// What each guest tick's edits answered, in order.
    static ANSWERS: RefCell<Vec<Result<i32, mod_api::PartStateError>>> = const { RefCell::new(Vec::new()) };
}

struct Counter;
impl mod_api::BlockPart for Counter {
    fn block_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[]
    }
}
impl BlockEntityPart for Counter {
    fn block_entity_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
        &[Some(mod_api::HookId::BlockEntityTick)]
    }
    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        let level = host.level(host.dimension()).unwrap();
        let entity = host.block_entity_identity(level, event.pos).unwrap();
        let edited = Counter::edit_state(host, entity, |_, n| {
            *n += 1;
            *n
        });
        // An aborted outer transaction takes an inner edit back: the next tick counts on from
        // this one's value, not from 100.
        let aborted: Result<(), ()> = host.transaction(|host, _| {
            Counter::edit_state(host, entity, |_, n| *n = 100).unwrap();
            Err(())
        });
        assert_eq!(aborted, Err(()));
        ANSWERS.with(|answers| answers.borrow_mut().push(edited));
    }
}
impl BlockEntityPartState for Counter {
    type Data = i32;
    const DATA: &'static AttachmentType<i32> = &TICKS;
}

#[test]
fn wire_loopback_part_state_edits_cross_and_keep_their_revision() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let index = guest::add_handler(vec![guest::view::<dyn BlockEntityPart>(Arc::new(Counter))]);
    let proxy = WasmHandler {
        index,
        module: WasmMod {
            group: 0,
            subscribed: &[],
            listeners: &[],
            corrupt: false,
        },
    };
    let level = LevelRef::new(
        mod_api::WorldEpoch::new(1),
        mod_api::DimensionKey::OVERWORLD,
    );
    let entity = mod_api::__private::game::block_entity_ref(level, [1, 64, 1], 4);
    let mut world = World {
        level,
        entity,
        store: Store::default(),
        tokens: 0,
    };
    let mut data = mod_api::ModBlockEntity::default();
    for _ in 0..3 {
        proxy.block_entity_tick(
            &mut world,
            &mut BlockEntityTickEvent {
                pos: [1, 64, 1],
                reply: &mut mod_api::BlockEntityTickReply::Changed,
                entity: mod_api::ModBlockEntityView::new(&mut data),
            },
        );
    }
    // Each edit found the value it stored last time under the same revision: the erased value
    // comes back to the guest as the allocation it stored, not a fresh copy.
    let answers = ANSWERS.with(|answers| answers.take());
    assert_eq!(answers, [Ok(1), Ok(2), Ok(3)]);
    let (_, encoded) = &world.store.values[&(
        mod_api::Holder::BlockEntity([1, 64, 1]),
        "test".to_string(),
        "ticks".to_string(),
    )];
    assert_eq!(
        encoded.as_ref().and_then(|saved| saved.get(&"n".into())),
        Some(&mod_api::Tag::Int(3))
    );
    assert!(world.store.journal.is_empty());
}
