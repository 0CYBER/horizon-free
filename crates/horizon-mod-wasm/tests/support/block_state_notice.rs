use mod_api::{BlockMetadata, BlockState, LevelHost, LevelSide, OnBlockStateChangeEvent};
use std::cell::RefCell;

pub struct World {
    stone: BlockState,
    air: BlockState,
    pub read: RefCell<Vec<[i32; 3]>>,
}
impl LevelHost for World {
    fn state_block(&self, state: BlockState) -> Option<&str> {
        if state == self.stone {
            Some("minecraft:stone")
        } else if state == self.air {
            Some("minecraft:air")
        } else {
            None
        }
    }
    fn is_loaded(&self, pos: [i32; 3]) -> bool {
        self.read.borrow_mut().push(pos);
        true
    }
}
impl World {
    pub fn new() -> Self {
        let properties = std::collections::BTreeMap::new();
        mod_api::__private::resolve_block_keys(
            [
                "minecraft:stone",
                "minecraft:air",
                "minecraft:water_cauldron",
                "wasm_fixture:part_state",
            ],
            |_| {
                horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                    .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        let metadata = mod_api::__private::block_definitions().unwrap();
        let state = |name| {
            BlockState::from(
                metadata
                    .of(mod_api::__private::part_tests::block_key(name), &[])
                    .unwrap(),
            )
        };
        Self {
            stone: state("minecraft:stone"),
            air: state("minecraft:air"),
            read: RefCell::new(Vec::new()),
        }
    }
    pub fn event(&self, predicted: bool) -> OnBlockStateChangeEvent {
        OnBlockStateChangeEvent {
            dimension: mod_api::LevelDimension::Overworld,
            pos: mod_api::BlockPos::new(1, 64, 1),
            state: self.stone,
            old: self.stone,
            new: self.air,
            predicted,
        }
    }
    pub fn check(&self, part: &dyn mod_api::BlockPart) {
        self.read.borrow_mut().clear();
        part.on_block_state_change(LevelSide::Client(self), &mut self.event(true));
        part.on_block_state_change(LevelSide::Client(self), &mut self.event(false));
        assert_eq!(*self.read.borrow(), [[1, 64, 1], [0, 64, 1]]);
    }
    pub fn dispatch(&self, predicted: bool) {
        mod_api::dispatch_on_block_state_change(
            LevelSide::Client(self),
            mod_api::LevelDimension::Overworld,
            mod_api::BlockPos::new(1, 64, 1),
            self.stone,
            self.stone,
            self.air,
            predicted,
        );
    }
}
