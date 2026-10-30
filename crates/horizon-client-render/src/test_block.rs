//! `TestBlockEntity`: the test block's mode, message and powered flag, with
//! its save format (`saveAdditional` / `loadAdditional`).
use horizon_client_core::nbt_value::{number, string, text};
use horizon_world::nbt::Tag;

/// `TestBlockMode`, in id order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestBlockMode {
    Start,
    Log,
    Fail,
    Accept,
}

impl TestBlockMode {
    pub const ALL: [Self; 4] = [Self::Start, Self::Log, Self::Fail, Self::Accept];
    /// `getSerializedName`: the block state's `mode` value.
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Log => "log",
            Self::Fail => "fail",
            Self::Accept => "accept",
        }
    }
    pub fn of_serialized_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.serialized_name() == name)
    }
    pub fn id(self) -> i32 {
        self as i32
    }
    /// `ByIdMap.continuous(.., OutOfBoundsStrategy.ZERO)`.
    pub fn by_id(id: i32) -> Self {
        usize::try_from(id)
            .ok()
            .and_then(|i| Self::ALL.get(i).copied())
            .unwrap_or(Self::Start)
    }
    /// `getDisplayName`'s lang key.
    pub fn label_key(self) -> String {
        format!("test_block.mode.{}", self.serialized_name())
    }
    /// `getDetailedMessage`'s lang key.
    pub fn info_key(self) -> String {
        format!("test_block.mode_info.{}", self.serialized_name())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestBlock {
    pub mode: TestBlockMode,
    pub message: String,
    pub powered: bool,
}

impl TestBlock {
    /// `new TestBlockEntity(pos, state)`: the mode is the state's.
    pub fn new(mode: TestBlockMode) -> Self {
        Self {
            mode,
            message: String::new(),
            powered: false,
        }
    }

    /// `saveAdditional` (also `getUpdateTag`, which is `saveCustomOnly`).
    pub fn save(&self) -> Vec<(&'static str, Tag)> {
        vec![
            ("mode", string(self.mode.serialized_name())),
            ("message", string(&self.message)),
            ("powered", Tag::Byte(i8::from(self.powered))),
        ]
    }

    /// `loadAdditional`: a missing or unknown mode is FAIL.
    pub fn load(tag: &Tag) -> Self {
        Self {
            mode: text(tag.get("mode"))
                .and_then(|m| TestBlockMode::of_serialized_name(&m))
                .unwrap_or(TestBlockMode::Fail),
            message: text(tag.get("message")).unwrap_or_default(),
            powered: number(tag.get("powered")).is_some_and(|v| v != 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_like_test_block_entity() {
        let block = TestBlock {
            mode: TestBlockMode::Accept,
            message: "all done".into(),
            powered: true,
        };
        let tag = Tag::compound(block.save());
        assert_eq!(text(tag.get("mode")).as_deref(), Some("accept"));
        assert_eq!(TestBlock::load(&tag), block);
    }

    #[test]
    fn load_defaults_to_fail_and_ids_wrap_to_start() {
        let block = TestBlock::load(&Tag::compound(Vec::<(&str, Tag)>::new()));
        assert_eq!(block, TestBlock::new(TestBlockMode::Fail));
        assert_eq!(TestBlockMode::by_id(3), TestBlockMode::Accept);
        assert_eq!(TestBlockMode::by_id(9), TestBlockMode::Start);
        assert_eq!(TestBlockMode::Log.id(), 1);
    }
}
