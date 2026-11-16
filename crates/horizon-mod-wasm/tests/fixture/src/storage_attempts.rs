//! Opt-in actual Game storage -> host notice -> same active guest reentry regression.
use mod_api::*;
use std::sync::atomic::{AtomicU32, Ordering};

const SOURCE: [i32; 3] = [8, 65, 8];
const DESTINATION: [i32; 3] = [8, 63, 8];
static ACTIVE: AtomicU32 = AtomicU32::new(0);
static PULL_NOTICES: AtomicU32 = AtomicU32::new(0);
static PUSH_NOTICES: AtomicU32 = AtomicU32::new(0);

pub fn register(r: &mut dyn Registrar) {
    let barrel = r.block_key("minecraft:barrel");
    r.block_part(barrel, Notices);
    r.block_capability_before_game(&ITEMS, &barrel, |_, at, _, _| {
        (at.pos == SOURCE || at.pos == DESTINATION)
            .then(|| Box::new(Storage(at)) as Box<dyn ResourceHandler<ItemResource>>)
    });
}

struct Storage(BlockSide);

impl Storage {
    fn entity(&self, host: &dyn Host) -> BlockEntityRef {
        host.block_entity(self.0.level, self.0.pos).unwrap()
    }
}

impl ResourceHandler<ItemResource> for Storage {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }
    fn resource(&self, host: &mut dyn ServerHost, index: usize) -> Option<ItemResource> {
        self.entity(host)
            .item_slot(host, index as u32)
            .unwrap()
            .as_ref()
            .map(ItemResource::of)
    }
    fn amount(&self, host: &mut dyn ServerHost, index: usize) -> u32 {
        self.entity(host)
            .item_slot(host, index as u32)
            .unwrap()
            .map_or(0, |stack| stack.count())
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
        64
    }
    fn insert(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> u32 {
        panic!("a hopper push must use the handler's indexless insert")
    }
    fn insert_any(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        _: &ItemResource,
        amount: u32,
    ) -> u32 {
        assert_eq!(self.0.pos, DESTINATION);
        assert_eq!(self.0.side, Some(Direction::Up));
        assert_eq!(amount, 1);
        assert_eq!(host.transaction_depth(tx), Some(0));
        let entity = self.entity(host);
        assert_eq!(entity.data_slots(host), Some(vec![0]));
        assert!(entity.set_data_slot(host, 0, 1).unwrap());
        let before = PUSH_NOTICES.load(Ordering::SeqCst);
        assert_eq!(ACTIVE.swap(2, Ordering::SeqCst), 0);
        assert!(entity.set_changed(host).unwrap());
        assert_eq!(
            PUSH_NOTICES.load(Ordering::SeqCst),
            before + 1,
            "same-guest notice must finish before insert returns"
        );
        ACTIVE.store(0, Ordering::SeqCst);
        assert_eq!(host.transaction_current().as_ref(), Some(tx));
        0
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        index: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        assert_eq!(self.0.pos, SOURCE);
        assert_eq!(self.0.side, Some(Direction::Down));
        assert_eq!((index, amount), (0, 1));
        assert_eq!(host.transaction_depth(tx), Some(0));
        let entity = self.entity(host);
        let stack = entity.item_slot(host, 0).unwrap().unwrap();
        assert_eq!(
            stack.count(),
            1,
            "the previous aborted extraction restored real Game storage"
        );
        assert_eq!(ItemResource::of(&stack), *resource);
        assert!(entity.set_item_slot(host, tx, 0, None).unwrap());
        let before = PULL_NOTICES.load(Ordering::SeqCst);
        assert_eq!(ACTIVE.swap(1, Ordering::SeqCst), 0);
        assert!(entity.set_changed(host).unwrap());
        assert_eq!(
            PULL_NOTICES.load(Ordering::SeqCst),
            before + 1,
            "same-guest notice must finish before extract returns"
        );
        ACTIVE.store(0, Ordering::SeqCst);
        assert_eq!(host.transaction_current().as_ref(), Some(tx));
        1
    }
}

struct Notices;

#[mod_api::horizon_part]
impl BlockPart for Notices {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let (name, active, notices) = if pos == SOURCE {
            ("pull", 1, &PULL_NOTICES)
        } else if pos == DESTINATION {
            ("push", 2, &PUSH_NOTICES)
        } else {
            return;
        };
        assert_eq!(
            ACTIVE.load(Ordering::SeqCst),
            active,
            "callback reenters the still-active storage handler"
        );
        assert_eq!(host.transaction_lifecycle(), TransactionLifecycle::Open);
        let entity = host.block_entity(event.level, pos).unwrap();
        let value = if active == 1 {
            assert_eq!(entity.item_slot(host, 0).unwrap(), None);
            0
        } else {
            assert_eq!(entity.data_slots(host), Some(vec![1]));
            let hopper = event
                .level
                .capability(host, &ITEMS, [8, 64, 8], None)
                .unwrap();
            assert_eq!(
                hopper.amount(host, 0),
                0,
                "source removal precedes insert's notice"
            );
            1
        };
        let call = notices.fetch_add(1, Ordering::SeqCst) + 1;
        host.messages()
            .broadcast(Text::literal(format!(
                "wasm storage {name} notice {call} value {value} Open"
            )))
            .unwrap();
    }
}
