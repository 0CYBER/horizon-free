mod api {
    pub enum RecipeBookIdentity {
        Crafting,
        Furnace,
        BlastFurnace,
        Smoker,
        Custom(String),
    }
}
