//! Item stacks on the wire (ItemStack.OPTIONAL_STREAM_CODEC): a count, the
//! item's registry id and a data component patch, plus the hashed form the
//! client sends back in container clicks (HashedStack).
use crate::{
    Error, Result,
    buf::{Codec, Reader, Writer},
};
use horizon_world::nbt::Tag;

/// minecraft:data_component_type registry ids (26.3).
pub mod component {
    pub const CUSTOM_DATA: i32 = 0;
    pub const MAX_STACK_SIZE: i32 = 1;
    pub const MAX_DAMAGE: i32 = 2;
    pub const DAMAGE: i32 = 3;
    pub const UNBREAKABLE: i32 = 4;
    pub const CUSTOM_NAME: i32 = 6;
    pub const ITEM_NAME: i32 = 9;
    pub const ITEM_MODEL: i32 = 10;
    pub const LORE: i32 = 11;
    pub const RARITY: i32 = 12;
    pub const ENCHANTMENTS: i32 = 13;
    pub const CUSTOM_MODEL_DATA: i32 = 17;
    pub const TOOLTIP_DISPLAY: i32 = 18;
    pub const REPAIR_COST: i32 = 19;
    pub const CREATIVE_SLOT_LOCK: i32 = 20;
    pub const ENCHANTMENT_GLINT_OVERRIDE: i32 = 21;
    pub const STORED_ENCHANTMENTS: i32 = 45;
    pub const DYED_COLOR: i32 = 47;
    pub const MAP_ID: i32 = 48;
    pub const MAP_POST_PROCESSING: i32 = 50;
    pub const WRITABLE_BOOK_CONTENT: i32 = 56;
    pub const WRITTEN_BOOK_CONTENT: i32 = 57;
}

/// A written book's content (`WrittenBookContent.STREAM_CODEC`): the
/// filtered variants of the title and pages are read and dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct WrittenBook {
    pub title: String,
    pub author: String,
    pub generation: i32,
    /// Each page's text component.
    pub pages: Vec<Tag>,
    pub resolved: bool,
}

/// The value of one patched data component. Components whose stream codec
/// Horizon does not implement make the whole stack (and packet) undecodable.
#[derive(Clone, Debug, PartialEq)]
pub enum Component {
    Int(i32),
    Unit,
    Bool(bool),
    Text(Tag),
    Nbt(Tag),
    Ident(String),
    Lore(Vec<Tag>),
    Enchantments(Vec<(i32, i32)>),
    CustomModelData {
        floats: Vec<f32>,
        flags: Vec<bool>,
        strings: Vec<String>,
        colors: Vec<i32>,
    },
    TooltipDisplay {
        hide: bool,
        hidden: Vec<i32>,
    },
    /// `WritableBookContent`: the pages (raw text).
    WritableBook(Vec<String>),
    WrittenBook(WrittenBook),
}

/// `Filterable.streamCodec`: the raw value, then an optional filtered one
/// (dropped).
fn filterable<T>(r: &mut Reader, value: impl Fn(&mut Reader) -> Result<T>) -> Result<T> {
    let raw = value(r)?;
    if r.bool()? {
        value(r)?;
    }
    Ok(raw)
}
impl Component {
    fn read(kind: i32, r: &mut Reader) -> Result<Self> {
        use component::*;
        Ok(match kind {
            MAX_STACK_SIZE | MAX_DAMAGE | DAMAGE | RARITY | REPAIR_COST | MAP_ID
            | MAP_POST_PROCESSING => Self::Int(r.varint()?),
            WRITABLE_BOOK_CONTENT => {
                let n = r.count(100)?;
                Self::WritableBook(
                    (0..n)
                        .map(|_| filterable(r, |r| r.string(1024)))
                        .collect::<Result<_>>()?,
                )
            }
            WRITTEN_BOOK_CONTENT => {
                let title = filterable(r, |r| r.string(32))?;
                let author = r.string(32767)?;
                let generation = r.varint()?;
                let n = r.count(r.remaining())?;
                let pages = (0..n)
                    .map(|_| filterable(r, |r| r.nbt()))
                    .collect::<Result<_>>()?;
                Self::WrittenBook(WrittenBook {
                    title,
                    author,
                    generation,
                    pages,
                    resolved: r.bool()?,
                })
            }
            DYED_COLOR => Self::Int(r.i32()?),
            UNBREAKABLE | CREATIVE_SLOT_LOCK => Self::Unit,
            ENCHANTMENT_GLINT_OVERRIDE => Self::Bool(r.bool()?),
            CUSTOM_NAME | ITEM_NAME => Self::Text(r.nbt()?),
            CUSTOM_DATA => Self::Nbt(r.nbt()?),
            ITEM_MODEL => Self::Ident(r.string(32767)?),
            LORE => {
                let n = r.count(256)?;
                Self::Lore((0..n).map(|_| r.nbt()).collect::<Result<_>>()?)
            }
            ENCHANTMENTS | STORED_ENCHANTMENTS => {
                let n = r.count(r.remaining())?;
                Self::Enchantments(
                    (0..n)
                        .map(|_| Ok((r.varint()?, r.varint()?)))
                        .collect::<Result<_>>()?,
                )
            }
            CUSTOM_MODEL_DATA => {
                let n = r.count(r.remaining())?;
                let floats = (0..n).map(|_| r.f32()).collect::<Result<_>>()?;
                let n = r.count(r.remaining())?;
                let flags = (0..n).map(|_| r.bool()).collect::<Result<_>>()?;
                let n = r.count(r.remaining())?;
                let strings = (0..n).map(|_| r.string(32767)).collect::<Result<_>>()?;
                let n = r.count(r.remaining())?;
                let colors = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
                Self::CustomModelData {
                    floats,
                    flags,
                    strings,
                    colors,
                }
            }
            TOOLTIP_DISPLAY => {
                let hide = r.bool()?;
                let n = r.count(r.remaining())?;
                Self::TooltipDisplay {
                    hide,
                    hidden: (0..n).map(|_| r.varint()).collect::<Result<_>>()?,
                }
            }
            other => return Err(Error::new(format!("unsupported item component {other}"))),
        })
    }
    fn write(&self, kind: i32, w: &mut Writer) {
        match self {
            Self::Int(v) if kind == component::DYED_COLOR => w.i32(*v),
            Self::Int(v) => w.varint(*v),
            Self::Unit => {}
            Self::Bool(v) => w.bool(*v),
            Self::Text(t) | Self::Nbt(t) => w.nbt(t),
            Self::Ident(s) => w.string(s),
            Self::Lore(lines) => {
                w.varint(lines.len() as i32);
                lines.iter().for_each(|l| w.nbt(l));
            }
            Self::Enchantments(list) => {
                w.varint(list.len() as i32);
                for (e, level) in list {
                    w.varint(*e);
                    w.varint(*level);
                }
            }
            Self::CustomModelData {
                floats,
                flags,
                strings,
                colors,
            } => {
                w.varint(floats.len() as i32);
                floats.iter().for_each(|f| w.f32(*f));
                w.varint(flags.len() as i32);
                flags.iter().for_each(|f| w.bool(*f));
                w.varint(strings.len() as i32);
                strings.iter().for_each(|s| w.string(s));
                w.varint(colors.len() as i32);
                colors.iter().for_each(|c| w.i32(*c));
            }
            Self::TooltipDisplay { hide, hidden } => {
                w.bool(*hide);
                w.varint(hidden.len() as i32);
                hidden.iter().for_each(|h| w.varint(*h));
            }
            Self::WritableBook(pages) => {
                w.varint(pages.len() as i32);
                for page in pages {
                    w.string(page);
                    w.bool(false);
                }
            }
            Self::WrittenBook(b) => {
                w.string(&b.title);
                w.bool(false);
                w.string(&b.author);
                w.varint(b.generation);
                w.varint(b.pages.len() as i32);
                for page in &b.pages {
                    w.nbt(page);
                    w.bool(false);
                }
                w.bool(b.resolved);
            }
        }
    }
}

/// A non-empty item stack.
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub item: i32,
    pub count: i32,
    pub added: Vec<(i32, Component)>,
    pub removed: Vec<i32>,
}
impl Stack {
    pub fn new(item: i32, count: i32) -> Self {
        Self {
            item,
            count,
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
    pub fn component(&self, kind: i32) -> Option<&Component> {
        self.added.iter().find(|(k, _)| *k == kind).map(|(_, c)| c)
    }
    pub fn damage(&self) -> i32 {
        match self.component(component::DAMAGE) {
            Some(Component::Int(d)) => *d,
            _ => 0,
        }
    }
}
/// ItemStack.OPTIONAL_STREAM_CODEC: `None` is the empty stack (count 0).
pub struct Slot;
impl Codec<Option<Stack>> for Slot {
    fn read(r: &mut Reader) -> Result<Option<Stack>> {
        let count = r.varint()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = r.varint()?;
        let positive = r.count(65536)?;
        let negative = r.count(65536)?;
        let mut added = Vec::with_capacity(positive.min(64));
        for _ in 0..positive {
            let kind = r.varint()?;
            added.push((kind, Component::read(kind, r)?));
        }
        let removed = (0..negative).map(|_| r.varint()).collect::<Result<_>>()?;
        Ok(Some(Stack {
            item,
            count,
            added,
            removed,
        }))
    }
    fn write(v: &Option<Stack>, w: &mut Writer) {
        let Some(s) = v else {
            w.varint(0);
            return;
        };
        w.varint(s.count);
        w.varint(s.item);
        w.varint(s.added.len() as i32);
        w.varint(s.removed.len() as i32);
        for (kind, c) in &s.added {
            w.varint(*kind);
            c.write(*kind, w);
        }
        for kind in &s.removed {
            w.varint(*kind);
        }
    }
}

/// ItemStackTemplate.STREAM_CODEC: the item, the count, then the patch.
pub struct Template;
impl Codec<Stack> for Template {
    fn read(r: &mut Reader) -> Result<Stack> {
        let item = r.varint()?;
        let count = r.varint()?;
        let positive = r.count(65536)?;
        let negative = r.count(65536)?;
        let mut added = Vec::with_capacity(positive.min(64));
        for _ in 0..positive {
            let kind = r.varint()?;
            added.push((kind, Component::read(kind, r)?));
        }
        let removed = (0..negative).map(|_| r.varint()).collect::<Result<_>>()?;
        Ok(Stack {
            item,
            count,
            added,
            removed,
        })
    }
    fn write(s: &Stack, w: &mut Writer) {
        w.varint(s.item);
        w.varint(s.count);
        w.varint(s.added.len() as i32);
        w.varint(s.removed.len() as i32);
        for (kind, c) in &s.added {
            w.varint(*kind);
            c.write(*kind, w);
        }
        for kind in &s.removed {
            w.varint(*kind);
        }
    }
}

/// HashedStack: what the client believes a slot holds. Component values are
/// sent as hashes; Horizon sends patches without hashes, so a server whose
/// stack carries components resynchronises that slot (as it does for any
/// mismatched prediction).
#[derive(Clone, Debug, PartialEq)]
pub struct Hashed {
    pub item: i32,
    pub count: i32,
    pub added: Vec<(i32, i32)>,
    pub removed: Vec<i32>,
}
impl Hashed {
    pub fn of(stack: &Stack) -> Self {
        Self {
            item: stack.item,
            count: stack.count,
            added: Vec::new(),
            removed: stack.removed.clone(),
        }
    }
}
pub struct HashedSlot;
impl Codec<Option<Hashed>> for HashedSlot {
    fn read(r: &mut Reader) -> Result<Option<Hashed>> {
        if !r.bool()? {
            return Ok(None);
        }
        let item = r.varint()?;
        let count = r.varint()?;
        let n = r.count(256)?;
        let added = (0..n)
            .map(|_| Ok((r.varint()?, r.i32()?)))
            .collect::<Result<_>>()?;
        let n = r.count(256)?;
        let removed = (0..n).map(|_| r.varint()).collect::<Result<_>>()?;
        Ok(Some(Hashed {
            item,
            count,
            added,
            removed,
        }))
    }
    fn write(v: &Option<Hashed>, w: &mut Writer) {
        w.bool(v.is_some());
        if let Some(h) = v {
            w.varint(h.item);
            w.varint(h.count);
            w.varint(h.added.len() as i32);
            for (k, hash) in &h.added {
                w.varint(*k);
                w.i32(*hash);
            }
            w.varint(h.removed.len() as i32);
            for k in &h.removed {
                w.varint(*k);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::literal;

    fn round<T: PartialEq + std::fmt::Debug, C: Codec<T>>(v: T) {
        let mut w = Writer::new();
        C::write(&v, &mut w);
        let mut r = Reader::new(&w.data);
        assert_eq!(C::read(&mut r).unwrap(), v);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn stacks_round_trip() {
        round::<Option<Stack>, Slot>(None);
        round::<Option<Stack>, Slot>(Some(Stack::new(42, 64)));
        let mut sword = Stack::new(900, 1);
        sword.added.push((component::DAMAGE, Component::Int(17)));
        sword.added.push((
            component::CUSTOM_NAME,
            Component::Text(literal("Excalibur")),
        ));
        sword.added.push((
            component::ENCHANTMENTS,
            Component::Enchantments(vec![(5, 3), (12, 1)]),
        ));
        sword
            .added
            .push((component::DYED_COLOR, Component::Int(-6265536)));
        sword.added.push((
            component::LORE,
            Component::Lore(vec![literal("a"), literal("b")]),
        ));
        sword.removed.push(component::REPAIR_COST);
        let mut book = Stack::new(1100, 1);
        book.added.push((
            component::WRITTEN_BOOK_CONTENT,
            Component::WrittenBook(WrittenBook {
                title: "Notes".into(),
                author: "Guest".into(),
                generation: 1,
                pages: vec![literal("one"), literal("two")],
                resolved: true,
            }),
        ));
        book.added
            .push((component::MAP_POST_PROCESSING, Component::Int(1)));
        round::<Option<Stack>, Slot>(Some(book));
        let mut quill = Stack::new(1099, 1);
        quill.added.push((
            component::WRITABLE_BOOK_CONTENT,
            Component::WritableBook(vec!["a".into(), String::new()]),
        ));
        round::<Option<Stack>, Slot>(Some(quill));
        assert_eq!(sword.damage(), 17);
        round::<Option<Stack>, Slot>(Some(sword.clone()));
        round::<Option<Hashed>, HashedSlot>(Some(Hashed::of(&sword)));
        round::<Option<Hashed>, HashedSlot>(None);
    }

    #[test]
    fn filtered_book_pages_are_read_and_dropped() {
        // A writable book with one page "hi" and its filtered "**".
        let bytes = [1, 7, 1, 0, 56, 1, 2, b'h', b'i', 1, 2, b'*', b'*'];
        let stack = Slot::read(&mut Reader::new(&bytes)).unwrap().unwrap();
        assert_eq!(
            stack.component(component::WRITABLE_BOOK_CONTENT),
            Some(&Component::WritableBook(vec!["hi".into()]))
        );
    }

    #[test]
    fn empty_stack_is_a_zero_count() {
        let mut w = Writer::new();
        Slot::write(&None, &mut w);
        assert_eq!(w.data, [0]);
        let mut w = Writer::new();
        Slot::write(&Some(Stack::new(1, 2)), &mut w);
        assert_eq!(w.data, [2, 1, 0, 0]);
    }

    #[test]
    fn unknown_components_are_errors() {
        let bytes = [1, 5, 1, 0, 99];
        assert!(Slot::read(&mut Reader::new(&bytes)).is_err());
    }
}
