//! Split entity-list borrows so level inventory reads can mutate carts.
macro_rules! borrow_level_list {
    (Cart, $game:expr, $($list:ident).+) => { &[] };
    ($kind:ident, $game:expr, $($list:ident).+) => { &$game$(.$list)+ };
}
pub(crate) use borrow_level_list;

macro_rules! borrow_lists {
    ((without_carts $game:expr) $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $crate::game::entity::EntityLists {
            $($field: $crate::game::entity::borrow_level_list!($kind, $game, $($list).+),)*
        }
    };
    (($game:expr) $($kind:ident($ty:ty) $field:ident: $($list:ident).+,)*) => {
        $crate::game::entity::EntityLists {
            $($field: &$game$(.$list)+,)*
        }
    };
}
pub(crate) use borrow_lists;

/// `$game`'s [`super::EntityLists`]; a macro so the borrow stays field by field.
macro_rules! entity_lists_of {
    ($game:expr, without_carts) => {
        $crate::game::entity::entity_lists!(borrow_lists, without_carts $game)
    };
    ($game:expr) => {
        $crate::game::entity::entity_lists!(borrow_lists, $game)
    };
}
pub(crate) use entity_lists_of;
