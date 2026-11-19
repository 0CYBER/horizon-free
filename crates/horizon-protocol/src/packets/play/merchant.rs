//! A villager's or wandering trader's trades: `ClientboundMerchantOffersPacket`.
use crate::{
    Error, Result,
    buf::{Bool, Codec, Reader, VarInt, Writer},
    item::{Component, Slot, Stack},
    packet,
};

/// ItemCost.STREAM_CODEC: item, count, then the exact component predicate
/// (a list of typed component values, with no patch removal count).
#[derive(Clone, Debug, PartialEq)]
pub struct NetCost {
    pub item: i32,
    pub count: i32,
    pub components: Vec<(i32, Component)>,
}
impl From<Stack> for NetCost {
    fn from(stack: Stack) -> Self {
        Self {
            item: stack.item,
            count: stack.count,
            components: stack.added,
        }
    }
}
impl From<NetCost> for Stack {
    fn from(cost: NetCost) -> Self {
        Self {
            item: cost.item,
            count: cost.count,
            added: cost.components,
            removed: Vec::new(),
            wire: Default::default(),
        }
    }
}
/// MerchantOffer.STREAM_CODEC.
#[derive(Clone, Debug, PartialEq)]
pub struct NetOffer {
    pub cost_a: NetCost,
    pub result: Stack,
    pub cost_b: Option<NetCost>,
    pub out_of_stock: bool,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}
pub struct Offers;
impl Codec<Vec<NetOffer>> for Offers {
    fn read(r: &mut Reader) -> Result<Vec<NetOffer>> {
        fn cost(r: &mut Reader) -> Result<NetCost> {
            let item = r.varint()?;
            let count = r.varint()?;
            let n = r.count(r.remaining())?;
            let mut components = Vec::with_capacity(n.min(64));
            for _ in 0..n {
                let kind = r.varint()?;
                components.push((kind, Component::read(kind, r)?));
            }
            Ok(NetCost {
                item,
                count,
                components,
            })
        }
        let n = r.count(r.remaining())?;
        let mut out = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            let cost_a = cost(r)?;
            let result = Slot::read(r)?.ok_or_else(|| Error::new("empty merchant result"))?;
            let cost_b = if r.bool()? { Some(cost(r)?) } else { None };
            out.push(NetOffer {
                cost_a,
                result,
                cost_b,
                out_of_stock: r.bool()?,
                uses: r.i32()?,
                max_uses: r.i32()?,
                xp: r.i32()?,
                special_price: r.i32()?,
                price_multiplier: r.f32()?,
                demand: r.i32()?,
            });
        }
        Ok(out)
    }
    fn write(v: &Vec<NetOffer>, w: &mut Writer) {
        fn cost(c: &NetCost, w: &mut Writer) {
            w.varint(c.item);
            w.varint(c.count);
            w.varint(c.components.len() as i32);
            for (kind, value) in &c.components {
                w.varint(*kind);
                value.write(*kind, w);
            }
        }
        w.varint(v.len() as i32);
        for o in v {
            cost(&o.cost_a, w);
            Slot::write(&Some(o.result.clone()), w);
            w.bool(o.cost_b.is_some());
            if let Some(b) = &o.cost_b {
                cost(b, w);
            }
            w.bool(o.out_of_stock);
            w.i32(o.uses);
            w.i32(o.max_uses);
            w.i32(o.xp);
            w.i32(o.special_price);
            w.f32(o.price_multiplier);
            w.i32(o.demand);
        }
    }
}
packet!(
    /// ClientboundMerchantOffersPacket.
    MerchantOffers {
        container: i32 => VarInt,
        offers: Vec<NetOffer> => Offers,
        level: i32 => VarInt,
        xp: i32 => VarInt,
        show_progress: bool => Bool,
        can_restock: bool => Bool,
    }
);

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.
impl crate::retained::OwnedHeap for NetCost {
    fn owned_heap_bytes(&self) -> usize {
        let Self {
            item,
            count,
            components,
        } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(item))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(count))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(components))
    }
}
impl crate::retained::OwnedHeap for NetOffer {
    fn owned_heap_bytes(&self) -> usize {
        let Self {
            cost_a,
            result,
            cost_b,
            out_of_stock,
            uses,
            max_uses,
            xp,
            special_price,
            price_multiplier,
            demand,
        } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(cost_a))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(result))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(cost_b))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(out_of_stock))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(uses))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(max_uses))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(xp))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(special_price))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(
                price_multiplier,
            ))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(demand))
    }
}
impl crate::retained::OwnedHeap for Offers {
    fn owned_heap_bytes(&self) -> usize {
        let Self = self;
        0usize
    }
}
// End generated retained traversals.
