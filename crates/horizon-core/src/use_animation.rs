#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UseAnimation {
    #[default]
    None,
    Eat,
    Drink,
    Block,
    Bow,
    Trident,
    Crossbow,
    Spyglass,
    TootHorn,
    Brush,
    Bundle,
    Spear,
}
