//! Retained allocation charges, excluding inline packet storage and allocator metadata.
//! Vec/String/Box storage uses actual capacity/layout. Shared Arc storage is charged in
//! full per reference (safe overcount), never scanned through unrelated registries.
use std::{collections::BTreeMap, sync::Arc};
/// Required owned-allocation traversal. There is deliberately no default implementation:
/// adding a field with a new representation must supply its accounting policy.
pub trait OwnedHeap {
    fn owned_heap_bytes(&self) -> usize;
}
macro_rules! scalar { ($($t:ty),*) => { $(impl OwnedHeap for $t { fn owned_heap_bytes(&self) -> usize { 0 } })* }; }
scalar!(
    (),
    bool,
    u8,
    i8,
    u16,
    i16,
    u32,
    i32,
    u64,
    i64,
    u128,
    i128,
    usize,
    isize,
    f32,
    f64
);
impl OwnedHeap for String {
    fn owned_heap_bytes(&self) -> usize {
        self.capacity()
    }
}
impl<T: OwnedHeap> OwnedHeap for [T] {
    fn owned_heap_bytes(&self) -> usize {
        self.iter()
            .fold(0usize, |sum, v| sum.saturating_add(v.owned_heap_bytes()))
    }
}
impl<T: OwnedHeap, const N: usize> OwnedHeap for [T; N] {
    fn owned_heap_bytes(&self) -> usize {
        self.as_slice().owned_heap_bytes()
    }
}
impl<T: OwnedHeap> OwnedHeap for Vec<T> {
    fn owned_heap_bytes(&self) -> usize {
        self.capacity()
            .saturating_mul(size_of::<T>())
            .saturating_add(self.as_slice().owned_heap_bytes())
    }
}
impl<T: OwnedHeap> OwnedHeap for Option<T> {
    fn owned_heap_bytes(&self) -> usize {
        self.as_ref().map_or(0, OwnedHeap::owned_heap_bytes)
    }
}
impl<T: OwnedHeap + ?Sized> OwnedHeap for Box<T> {
    fn owned_heap_bytes(&self) -> usize {
        size_of_val(&**self).saturating_add((**self).owned_heap_bytes())
    }
}
impl<T: OwnedHeap + ?Sized> OwnedHeap for Arc<T> {
    fn owned_heap_bytes(&self) -> usize {
        size_of_val(&**self)
            .saturating_add(2 * size_of::<usize>())
            .saturating_add(2 * (align_of_val(&**self).max(align_of::<usize>()) - 1))
            .saturating_add((**self).owned_heap_bytes())
    }
}
macro_rules! tuple { ($($t:ident:$i:tt),*) => { impl<$($t:OwnedHeap),*> OwnedHeap for ($($t,)*) { fn owned_heap_bytes(&self) -> usize { 0usize $(.saturating_add(self.$i.owned_heap_bytes()))* } } }; }
tuple!(A:0,B:1);
tuple!(A:0,B:1,C:2);
tuple!(A:0,B:1,C:2,D:3);
tuple!(A:0,B:1,C:2,D:3,E:4);
impl OwnedHeap for horizon_core::nbt::JavaString {
    fn owned_heap_bytes(&self) -> usize {
        self.0.owned_heap_bytes()
    }
}
impl OwnedHeap for horizon_core::nbt::Tag {
    fn owned_heap_bytes(&self) -> usize {
        use horizon_core::nbt::Tag::*;
        match self {
            ByteArray(v) => v.owned_heap_bytes(),
            String(v) => v.owned_heap_bytes(),
            List(v) => v.owned_heap_bytes(),
            Compound(v) => v.owned_heap_bytes(),
            IntArray(v) => v.owned_heap_bytes(),
            LongArray(v) => v.owned_heap_bytes(),
            End | Byte(_) | Short(_) | Int(_) | Long(_) | Float(_) | Double(_) => 0,
        }
    }
}
impl<K: OwnedHeap, V: OwnedHeap> OwnedHeap for BTreeMap<K, V> {
    fn owned_heap_bytes(&self) -> usize {
        // Audited Rust 1.99.1 alloc/collections/btree/node.rs: B=6, capacity=11,
        // non-root minimum=5, leaf header=(parent pointer,u16 index,u16 len),
        // internal nodes add 12 child pointers. Charge every node as internal.
        // At most 1 + len/5 nodes, including a possibly retained empty root.
        // Eight alignment gaps bound both layouts regardless of Rust field ordering.
        let align = align_of::<K>()
            .max(align_of::<V>())
            .max(align_of::<usize>());
        let node = 13usize
            .saturating_mul(size_of::<usize>())
            .saturating_add(4)
            .saturating_add(11usize.saturating_mul(size_of::<K>().saturating_add(size_of::<V>())))
            .saturating_add(8usize.saturating_mul(align - 1));
        self.iter()
            .fold((1 + self.len() / 5).saturating_mul(node), |sum, (k, v)| {
                sum.saturating_add(k.owned_heap_bytes())
                    .saturating_add(v.owned_heap_bytes())
            })
    }
}

/// Conservative requested storage for a fresh, no-erasure std hash table.
///
/// `capacity` must be the public capacity of a table built only by allocation,
/// insertion/replacement and reservation, with no erasure/tombstone history.
/// This is NOT a bound for arbitrary HashMap/HashSet histories. Pass the pair
/// `(K, V)` layout for a map, or the element layout for a set. Inline table
/// storage, owned key/value descendants and allocator metadata are excluded.
///
/// Audited Rust 1.99.1 / hashbrown 0.17.1: at least half the buckets are usable;
/// table storage is aligned element slots, one control byte per bucket and at
/// most 16 mirrored control bytes. `max(align(T), 16) - 1` bounds padding before
/// controls. Twice fresh capacity bounds the physical bucket count, including
/// small tables. Empty fresh tables have no allocation. See the std audit tool.
pub fn fresh_hash_table_bytes<T>(capacity: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    capacity
        .saturating_mul(2)
        .saturating_mul(size_of::<T>().saturating_add(1))
        .saturating_add(align_of::<T>().max(16) - 1)
        .saturating_add(16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::{custom_payload::CustomPayload, play};
    use horizon_core::nbt::{JavaString, Tag};
    #[test]
    fn packet_pressure_includes_spare_channel_and_body_capacity() {
        let mut channel = String::with_capacity(160);
        channel.push_str("test:raw");
        let body = Vec::with_capacity(24000);
        let expected = channel.capacity() + body.capacity();
        let packet = play::Clientbound::CustomPayload(CustomPayload {
            channel,
            data: body,
        });
        assert_eq!(packet.owned_heap_bytes(), expected);
        let unknown = play::Serverbound::Unknown(999, Vec::with_capacity(8000));
        assert_eq!(unknown.owned_heap_bytes(), 8000);
    }
    #[test]
    fn nested_nbt_counts_capacity_at_each_owned_layer() {
        let mut units = Vec::with_capacity(100);
        units.push(65);
        let mut children = Vec::with_capacity(20);
        children.push(Tag::String(JavaString(units)));
        children.push(Tag::ByteArray(Vec::with_capacity(4000)));
        let expected = children.capacity() * size_of::<Tag>() + 200 + 4000;
        let packet = play::Clientbound::BlockEntityData(play::BlockEntityData {
            pos: [0; 3],
            kind: 0,
            tag: Tag::List(children),
        });
        assert_eq!(packet.owned_heap_bytes(), expected);
    }
    #[test]
    fn opaque_map_bound_covers_empty_roots_and_insert_remove_histories() {
        let fresh = BTreeMap::<JavaString, Tag>::new();
        let mut churned = BTreeMap::new();
        for i in 0..2000 {
            churned.insert(
                JavaString::from(i.to_string().as_str()),
                Tag::ByteArray(vec![0; 10]),
            );
        }
        for i in 0..2000 {
            churned.remove(&JavaString::from(i.to_string().as_str()));
        }
        assert_eq!(churned.len(), 0);
        assert!(fresh.owned_heap_bytes() > 0); // Possible retained empty root charged even if absent.
        assert_eq!(fresh.owned_heap_bytes(), churned.owned_heap_bytes());
        let root_bound = fresh.owned_heap_bytes();
        for i in 0..17 {
            churned.insert(
                JavaString::from(i.to_string().as_str()),
                Tag::ByteArray(Vec::with_capacity(100)),
            );
        }
        let child_allocations: usize = churned
            .iter()
            .map(|(k, v)| k.owned_heap_bytes() + v.owned_heap_bytes())
            .sum();
        assert_eq!(
            churned.owned_heap_bytes(),
            4 * root_bound + child_allocations
        );
        assert!(churned.owned_heap_bytes() >= 1700);
    }
}
