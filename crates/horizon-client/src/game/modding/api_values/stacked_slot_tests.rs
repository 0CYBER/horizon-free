use super::{self as api_values, EngineStack, stack_limit_tests};
use crate::{items::Items, menu::SlotView};
use mod_api::{Flow, Hooks, ModEntry};
use mod_platform::testing::with_mods;

fn context() -> horizon_world::native_load_context::LoopContext {
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
    let inputs = std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default()));
    LoadContext::new(DataLoadOrigin::ServerDataLoad, inputs, [] as [&str; 0])
        .loop_context(LoopSide::Server, [] as [&str; 0])
}

struct Slot<'a> {
    items: &'a Items,
    value: Option<EngineStack>,
    writes: usize,
}
impl SlotView for Slot<'_> {
    fn with_staged(&mut self, edit: &mut dyn FnMut(&mut dyn SlotView) -> bool) -> bool {
        let mut staged = Self {
            items: self.items,
            value: self.value.clone(),
            writes: self.writes,
        };
        if !edit(&mut staged) {
            return false;
        }
        self.value = staged.value;
        self.writes = staged.writes;
        true
    }
    fn items(&self) -> &Items {
        self.items
    }
    fn item(&self) -> Option<EngineStack> {
        self.value.clone()
    }
    fn may_place(&self, _: &EngineStack) -> bool {
        true
    }
    fn may_pickup(&self) -> bool {
        true
    }
    fn allow_modification(&self) -> bool {
        true
    }
    fn safe_insert(&mut self, value: Option<EngineStack>, count: u32) -> Option<EngineStack> {
        assert_eq!(count, 1);
        assert!(self.value.is_none());
        self.value = value;
        self.writes += 1;
        None
    }
    fn safe_take(&mut self, count: u32, max: u32) -> Option<EngineStack> {
        assert_eq!((count, max), (1, 1));
        self.writes += 1;
        self.value.take()
    }
}
struct Listener {
    take: bool,
    invalid: bool,
}
#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn item_stacked_on_other(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::ItemStackedOnOtherEvent<'_, '_>,
    ) -> Flow {
        let mut stack = if self.take {
            event.event.clicked.safe_take(1, 1).unwrap()
        } else {
            let stack = event.event.carried.get().unwrap().clone();
            assert!(
                event
                    .event
                    .clicked
                    .safe_insert(Some(stack.clone()), 1)
                    .is_none()
            );
            stack
        };
        if self.invalid {
            stack.set_count(65).unwrap();
        }
        event.event.carried.set(if self.invalid || self.take {
            Some(stack)
        } else {
            None
        });
        event.event.cancellation_result = true;
        Flow::Cancel
    }
}

#[test]
fn invalid_cursor_rolls_back_insert_and_take_without_item_duplication() {
    for take in [false, true] {
        let items = stack_limit_tests::catalog();
        let original = EngineStack::new("minecraft:stick", 1);
        let mut slot = Slot {
            items: &items,
            value: take.then(|| original.clone()),
            writes: 0,
        };
        let mut cursor = (!take).then(|| original.clone());
        let before_slot = slot.value.clone();
        let before_cursor = cursor.clone();
        let context = context();
        let mut canceled = false;
        with_mods(
            vec![ModEntry::new(
                "invalid_cursor",
                Box::new(Listener {
                    take,
                    invalid: true,
                }),
            )]
            .into(),
            || {
                let mut event = crate::menu::StackedOnOther {
                    player: None,
                    player_entity_id: None,
                    slot: 0,
                    container: 0,
                    container_slot: 0,
                    primary: true,
                    carried: crate::menu::CarriedAccess(&mut cursor),
                    clicked: &mut slot,
                    cancellation_result: false,
                };
                api_values::stacked_on_other(&context, &mut event, &mut canceled);
                assert!(!event.cancellation_result);
            },
        );
        assert_eq!(cursor, before_cursor);
        assert_eq!(slot.value, before_slot);
        assert_eq!(slot.writes, 0);
        assert!(!canceled);
    }
}

#[test]
fn valid_cursor_commits_slot_writes_and_flags_together() {
    for take in [false, true] {
        let items = stack_limit_tests::catalog();
        let original = EngineStack::new("minecraft:stick", 1);
        let mut slot = Slot {
            items: &items,
            value: take.then(|| original.clone()),
            writes: 0,
        };
        let mut cursor = (!take).then(|| original.clone());
        let context = context();
        let mut canceled = false;
        with_mods(
            vec![ModEntry::new(
                "valid_cursor",
                Box::new(Listener {
                    take,
                    invalid: false,
                }),
            )]
            .into(),
            || {
                let mut event = crate::menu::StackedOnOther {
                    player: None,
                    player_entity_id: None,
                    slot: 0,
                    container: 0,
                    container_slot: 0,
                    primary: true,
                    carried: crate::menu::CarriedAccess(&mut cursor),
                    clicked: &mut slot,
                    cancellation_result: false,
                };
                api_values::stacked_on_other(&context, &mut event, &mut canceled);
                assert!(event.cancellation_result);
            },
        );
        assert_eq!(cursor, take.then(|| original.clone()));
        assert_eq!(slot.value, (!take).then_some(original));
        assert_eq!(slot.writes, 1);
        assert!(canceled);
    }
}
