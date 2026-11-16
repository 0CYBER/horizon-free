#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum EquipmentSlot {
    MainHand,
    OffHand,
    Feet,
    Legs,
    Chest,
    Head,
    Body,
    Saddle,
}
impl EquipmentSlot {
    pub const ALL: [Self; 8] = [
        Self::MainHand,
        Self::OffHand,
        Self::Feet,
        Self::Legs,
        Self::Chest,
        Self::Head,
        Self::Body,
        Self::Saddle,
    ];
    pub fn inventory_slot(self) -> Option<usize> {
        match self {
            Self::MainHand => None,
            Self::OffHand => Some(40),
            Self::Feet => Some(36),
            Self::Legs => Some(37),
            Self::Chest => Some(38),
            Self::Head => Some(39),
            Self::Body => Some(41),
            Self::Saddle => Some(42),
        }
    }
    pub fn from_inventory_slot(index: i32) -> Option<Self> {
        match index {
            36 => Some(Self::Feet),
            37 => Some(Self::Legs),
            38 => Some(Self::Chest),
            39 => Some(Self::Head),
            40 => Some(Self::OffHand),
            41 => Some(Self::Body),
            42 => Some(Self::Saddle),
            _ => None,
        }
    }
}
