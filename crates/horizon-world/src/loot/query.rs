//! Shared queried parameters and NeoForge conditions for both loot evaluators.
use super::LootStack;
pub use horizon_core::UNKNOWN_LOOT_TABLE;
use std::{cell::RefCell, rc::Rc};

/// Parameters shared unchanged by all nested tables. The first id set wins.
#[derive(Default)]
pub struct LootQuery {
    id: RefCell<Option<String>>,
    /// The stack whose data is being queried. No existing engine caller sets it.
    pub queried_stack: Option<LootStack>,
    /// Tool retained by the world evaluator and its nested tables.
    pub tool: Option<LootStack>,
}
impl LootQuery {
    /// An unset query with stack parameters retained for every nested table.
    pub fn new(queried_stack: Option<LootStack>, tool: Option<LootStack>) -> Self {
        Self {
            id: RefCell::new(None),
            queried_stack,
            tool,
        }
    }
    /// Set once, either by the caller or after the root completes.
    pub fn set_id(&self, id: &str) {
        self.id.borrow_mut().get_or_insert_with(|| id.to_owned());
    }
    /// The stored id, if a caller or completed root supplied one.
    pub fn id(&self) -> Option<String> {
        self.id.borrow().clone()
    }
    /// Unset contexts always report the unknown-table fallback.
    pub fn table_id(&self) -> String {
        self.id().unwrap_or_else(|| UNKNOWN_LOOT_TABLE.into())
    }
    /// Root-only completion point, after all nested tables. Nested rolls never call this.
    pub fn finish(&self, id: Option<&str>) {
        self.set_id(id.unwrap_or(UNKNOWN_LOOT_TABLE));
        // arismoko/horizon#1270: the global loot modifier pass belongs here.
    }
}
thread_local! { static QUERY: RefCell<Option<Rc<LootQuery>>> = const { RefCell::new(None) }; }
/// Supply parameters and inspect the root's completed queried id. Restores on unwind.
pub fn with_query<T>(query: Rc<LootQuery>, roll: impl FnOnce() -> T) -> T {
    struct Restore(Option<Rc<LootQuery>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            QUERY.with(|q| *q.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(QUERY.with(|q| q.replace(Some(query))));
    roll()
}
/// A fresh context unless a caller explicitly supplied query parameters.
pub fn current_query() -> Rc<LootQuery> {
    QUERY.with(|q| q.borrow().clone()).unwrap_or_default()
}
/// Recognise stock conditions before the mod-kind fallback, without drawing randomness.
pub fn condition(
    value: &serde_json::Value,
    query: &LootQuery,
    tool: Option<&LootStack>,
    default: impl Fn(&str, &str) -> bool,
) -> Option<bool> {
    match value["type"].as_str()? {
        "neoforge:loot_table_id" => Some(
            value["loot_table_id"]
                .as_str()
                .is_some_and(|id| id == query.id.borrow().as_deref().unwrap_or(UNKNOWN_LOOT_TABLE)),
        ),
        "neoforge:can_item_perform_ability" => Some(
            tool.zip(value["ability"].as_str())
                .is_some_and(|(tool, ability)| {
                    let mut holds = default(&tool.item, ability);
                    crate::native_load_hooks::loot_ability(tool, ability, &mut holds);
                    holds
                }),
        ),
        _ => None,
    }
}

/// Run a named root with one shared query, completing it after every nested table.
pub fn root<T>(id: &str, roll: impl FnOnce() -> T) -> T {
    let query = current_query();
    with_query(query.clone(), || {
        let result = roll();
        query.finish(Some(id));
        result
    })
}
