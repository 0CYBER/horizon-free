//! The server's replacement recipe property sets and stonecutter choices.
use super::{HolderSet, SlotDisplay};
use crate::{
    Result,
    buf::{Codec, Reader, Writer},
};

/// ClientboundUpdateRecipesPacket. Each packet replaces the previous generation,
/// including when either collection is empty.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateRecipes {
    /// Resource location and registry item IDs for each property set.
    pub properties: Vec<(String, Vec<i32>)>,
    /// SingleInputSet entries, in the server's recipe order.
    pub stonecutter: Vec<(HolderSet, SlotDisplay)>,
}
impl Codec<UpdateRecipes> for UpdateRecipes {
    fn read(r: &mut Reader) -> Result<Self> {
        let count = r.count(r.remaining())?;
        let mut properties = Vec::with_capacity(count);
        for _ in 0..count {
            let key = r.string(32767)?;
            let count = r.count(r.remaining())?;
            let items = (0..count)
                .map(|_| r.varint().map(crate::adapter::ids::item))
                .collect::<Result<_>>()?;
            properties.push((key, items));
        }
        let count = r.count(r.remaining())?;
        let stonecutter = (0..count)
            .map(|_| Ok((HolderSet::read(r)?, SlotDisplay::read(r)?)))
            .collect::<Result<_>>()?;
        Ok(Self {
            properties,
            stonecutter,
        })
    }
    fn write(v: &Self, w: &mut Writer) {
        w.varint(v.properties.len() as i32);
        for (key, items) in &v.properties {
            w.string(key);
            w.varint(items.len() as i32);
            for id in items {
                w.varint(*id);
            }
        }
        w.varint(v.stonecutter.len() as i32);
        for (input, display) in &v.stonecutter {
            input.write(w);
            SlotDisplay::write(display, w);
        }
    }
}

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.
impl crate::retained::OwnedHeap for UpdateRecipes {
    fn owned_heap_bytes(&self) -> usize {
        let Self {
            properties,
            stonecutter,
        } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(properties))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(stonecutter))
    }
}
// End generated retained traversals.

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_custom_sets_and_the_stonecutter_tail() {
        // One custom property, two items, one stonecutter choice: explicit
        // holder set [4], empty output display (type 0).
        let bytes = b"\x01\x09mod:input\x02\x04\x07\x01\x02\x04\x00";
        let mut r = Reader::new(bytes);
        let packet = UpdateRecipes::read(&mut r).unwrap();
        assert_eq!(packet.properties, vec![("mod:input".into(), vec![4, 7])]);
        assert_eq!(
            packet.stonecutter,
            vec![(HolderSet::Items(vec![4]), SlotDisplay::Empty)]
        );
        assert_eq!(r.remaining(), 0);
        let mut w = Writer::new();
        UpdateRecipes::write(&packet, &mut w);
        assert_eq!(w.data, bytes);
        for end in 0..bytes.len() {
            assert!(UpdateRecipes::read(&mut Reader::new(&bytes[..end])).is_err());
        }
    }
    #[test]
    fn empty_replacement_and_impossible_lengths() {
        assert_eq!(
            UpdateRecipes::read(&mut Reader::new(&[0, 0])).unwrap(),
            UpdateRecipes::default()
        );
        for bytes in [
            &[255, 255, 255, 255, 15][..],
            &[1, 0, 127][..],
            &[0, 127][..],
            // One stonecutter entry with i32::MIN as its holder-set count.
            &[0, 1, 128, 128, 128, 128, 8][..],
            // Smithing trim display with i32::MIN as its pattern holder.
            &[0, 1, 1, 8, 0, 0, 128, 128, 128, 128, 8][..],
        ] {
            assert!(UpdateRecipes::read(&mut Reader::new(bytes)).is_err());
        }
    }
}
