//! Active item ownership retained across a mutable listener dispatch.
use super::Items;
use std::{ops::Deref, sync::Arc};

/// Retain the selected owner without copying its item registry.
pub(in crate::game) enum ItemSnapshot {
    Shared(Arc<Items>),
    Resources(Arc<super::super::Resources>),
}
impl ItemSnapshot {
    fn capture(
        server: Option<&Arc<Items>>,
        world: Option<&Arc<Items>>,
        base: impl FnOnce() -> Self,
    ) -> Self {
        match server.or(world) {
            Some(items) => Self::Shared(Arc::clone(items)),
            None => base(),
        }
    }
}
impl Deref for ItemSnapshot {
    type Target = Items;
    fn deref(&self) -> &Items {
        match self {
            Self::Shared(items) => items,
            Self::Resources(resources) => resources.items(),
        }
    }
}
impl super::super::Game<'_> {
    pub(in crate::game) fn stack_boundary_items(&self) -> Option<ItemSnapshot> {
        Some(ItemSnapshot::capture(
            self.server_items.as_ref(),
            self.world_tables.as_ref().map(|tables| &tables.items),
            || ItemSnapshot::Resources(Arc::clone(&self.res)),
        ))
    }
}

impl<'a> super::super::GameLevel<'a> {
    pub(in crate::game) fn stack_boundary_items(&self) -> Option<&'a Items> {
        Some(self.items)
    }
}

/// Registry access for the remaining generated value conversions.
pub(in crate::game) trait ItemBoundary {
    fn boundary_items(&self) -> Option<&Items>;
}
impl ItemBoundary for super::super::Game<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        Some(self.items())
    }
}
impl ItemBoundary for super::super::GameLevel<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        Some(self.items)
    }
}
impl ItemBoundary for super::super::native_hooks::HitLevel<'_> {
    fn boundary_items(&self) -> Option<&Items> {
        self.0.tags()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_registry_limits_survive_world_replacement_and_owner_drop() {
        use super::super::{ApiStack, StackWriteError, TryIntoEngine, stack_limit_tests};

        fn catalog(maximum: u32) -> Items {
            let mut items = stack_limit_tests::catalog();
            items.items.get_mut("minecraft:stick").unwrap().max_stack = maximum;
            items
        }

        let base = Arc::new(catalog(4));
        let mut world = Arc::new(catalog(8));
        let server = Arc::new(catalog(16));
        let mut stack = ApiStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        stack.set_count(12).unwrap();

        let snapshot = ItemSnapshot::capture(Some(&server), Some(&world), || {
            ItemSnapshot::Shared(Arc::clone(&base))
        });
        drop(server);
        assert_eq!(stack.try_into_engine(&snapshot).unwrap().count, 12);

        let snapshot = ItemSnapshot::capture(None, Some(&world), || {
            ItemSnapshot::Shared(Arc::clone(&base))
        });
        world = Arc::new(catalog(32));
        assert_eq!(
            stack.try_into_engine(&snapshot),
            Err(StackWriteError::CountExceeded { maximum: 8 })
        );
        drop(snapshot);
        let snapshot = ItemSnapshot::capture(None, Some(&world), || {
            ItemSnapshot::Shared(Arc::clone(&base))
        });
        drop(world);
        assert_eq!(stack.try_into_engine(&snapshot).unwrap().count, 12);

        let snapshot =
            ItemSnapshot::capture(None, None, || ItemSnapshot::Shared(Arc::clone(&base)));
        drop(base);
        assert_eq!(
            stack.try_into_engine(&snapshot),
            Err(StackWriteError::CountExceeded { maximum: 4 })
        );
    }
}
