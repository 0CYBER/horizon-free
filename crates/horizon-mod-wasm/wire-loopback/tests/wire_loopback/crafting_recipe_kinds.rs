//! The recipe owner, opaque value and full grid survive generated resource dispatch.
use super::*;
use mod_api::{CraftingGrid, CraftingRecipeHandler, CraftingRecipeValue};
struct Diagonal;
impl CraftingRecipeHandler for Diagonal {
    fn decode(&self, json: &str) -> Result<CraftingRecipeValue, String> {
        if json == r#"{"count":3}"# {
            Ok(CraftingRecipeValue::new(vec![3, 0, 0, 0]))
        } else {
            Err("count must be 3".into())
        }
    }
    fn write(&self, value: &CraftingRecipeValue) -> Result<String, String> {
        assert_eq!(value.bytes, [3, 0, 0, 0]);
        Ok(r#"{"count":3}"#.into())
    }
    fn matches(&self, value: &CraftingRecipeValue, grid: CraftingGrid) -> bool {
        value.bytes == [3, 0, 0, 0]
            && grid.width == 3
            && grid.height == 3
            && grid
                .cells
                .iter()
                .enumerate()
                .all(|(i, c)| c.is_some() == [0, 4, 8].contains(&i))
    }
    fn assemble(&self, _: &CraftingRecipeValue, _: CraftingGrid) -> Option<mod_api::ItemStack> {
        mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 3)
    }
    fn remaining_items(
        &self,
        _: &CraftingRecipeValue,
        _: CraftingGrid,
    ) -> Option<Vec<Option<mod_api::ItemStack>>> {
        let mut out = vec![None; 9];
        out[4] = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 1);
        Some(out)
    }
}
#[test]
fn wire_loopback_crafting_kind_owner_value_grid_and_remainders() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let mut out = Writer::new();
    {
        let _resources = guest::resource_scope();
        let handler: Arc<dyn CraftingRecipeHandler> = Arc::new(Diagonal);
        out.put(&handler);
    }
    assert_eq!(out.error(), None);
    let module = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mut cx = boundary::Cx::default();
    cx.module = &module;
    let _resources = unsafe { boundary::resource_scope(&mut cx, true) };
    let handler = Reader::decode::<Arc<dyn CraftingRecipeHandler>>(out.bytes()).unwrap();
    let value = handler.decode(r#"{"count":3}"#).unwrap();
    assert_eq!(value.bytes, [3, 0, 0, 0]);
    assert_eq!(handler.write(&value).unwrap(), r#"{"count":3}"#);
    let key = REGISTERED_ITEM.get().unwrap();
    let mut cells = vec![None; 9];
    for i in [0, 4, 8] {
        cells[i] = mod_api::ItemStack::new(key, 2);
    }
    let grid = CraftingGrid::new(3, 3, cells).unwrap();
    assert!(handler.matches(&value, grid.clone()));
    assert_eq!(
        handler.assemble(&value, grid.clone()),
        mod_api::ItemStack::new(key, 3)
    );
    let mut expected = vec![None; 9];
    expected[4] = mod_api::ItemStack::new(key, 1);
    assert_eq!(handler.remaining_items(&value, grid), Some(expected));
    assert_eq!(handler.decode("{"), Err("count must be 3".into()));
}
