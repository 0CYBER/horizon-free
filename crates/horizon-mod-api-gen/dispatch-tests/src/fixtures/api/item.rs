pub mod item {
    pub mod menu {
        pub struct MenuButtonContext {
            pub player: u128,
            pub player_entity_id: i32,
            pub pos: [i32; 3],
            pub id: i32,
        }
        pub struct AnvilCraft;
        pub struct AnvilUpdate;
        pub struct GrindstonePlace;
        pub struct GrindstoneTake;
    }
    pub mod stations {
        pub struct EnchantmentRow<'a>(std::marker::PhantomData<&'a ()>);
    }
}
