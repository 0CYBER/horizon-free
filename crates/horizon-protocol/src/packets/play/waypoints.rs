//! The locator bar's waypoints: `ClientboundTrackedWaypointPacket`.
use crate::{
    Error, Result,
    buf::{Codec, Ident, Reader, Writer},
};

/// ClientboundTrackedWaypointPacket.Operation, by ordinal.
pub mod waypoint_operation {
    pub const TRACK: i32 = 0;
    pub const UNTRACK: i32 = 1;
    pub const UPDATE: i32 = 2;
}

/// TrackedWaypoint's identifier: `Either<UUID, String>`.
#[derive(Clone, Debug, PartialEq)]
pub enum WaypointId {
    Uuid(u128),
    Name(String),
}

/// What a TrackedWaypoint points at (its `Type` and contents).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaypointTarget {
    /// `TrackedWaypoint.empty`: an untracked waypoint.
    Empty,
    /// A block position (`Vec3iWaypoint`).
    Block([i32; 3]),
    /// A chunk (`ChunkWaypoint`).
    Chunk([i32; 2]),
    /// An angle in radians (`AzimuthWaypoint`).
    Azimuth(f32),
}

/// ClientboundTrackedWaypointPacket: the locator bar's waypoints.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedWaypoint {
    /// `waypoint_operation`.
    pub operation: i32,
    pub id: WaypointId,
    /// Waypoint.Icon: the style (a `waypoint_style_asset` key) and an RGB colour.
    pub style: String,
    pub color: Option<i32>,
    pub target: WaypointTarget,
}

impl Codec<TrackedWaypoint> for TrackedWaypoint {
    fn read(r: &mut Reader) -> Result<TrackedWaypoint> {
        let operation = r.varint()?;
        if !(0..=2).contains(&operation) {
            return Err(Error::new(format!("waypoint operation {operation}")));
        }
        let id = if r.bool()? {
            WaypointId::Uuid(r.uuid()?)
        } else {
            WaypointId::Name(r.string(32767)?)
        };
        let style = Ident::read(r)?;
        // ByteBufCodecs.RGB_COLOR: three bytes, read back as an opaque ARGB colour.
        let color = if r.bool()? {
            let [red, green, blue] = [r.u8()?, r.u8()?, r.u8()?].map(i32::from);
            Some(-16_777_216 | red << 16 | green << 8 | blue)
        } else {
            None
        };
        let target = match r.varint()? {
            0 => WaypointTarget::Empty,
            1 => WaypointTarget::Block([r.varint()?, r.varint()?, r.varint()?]),
            2 => WaypointTarget::Chunk([r.varint()?, r.varint()?]),
            3 => WaypointTarget::Azimuth(r.f32()?),
            other => return Err(Error::new(format!("waypoint type {other}"))),
        };
        Ok(TrackedWaypoint {
            operation,
            id,
            style,
            color,
            target,
        })
    }
    fn write(v: &TrackedWaypoint, w: &mut Writer) {
        w.varint(v.operation);
        match &v.id {
            WaypointId::Uuid(uuid) => {
                w.bool(true);
                w.uuid(*uuid);
            }
            WaypointId::Name(name) => {
                w.bool(false);
                w.string(name);
            }
        }
        w.string(&v.style);
        w.bool(v.color.is_some());
        if let Some(color) = v.color {
            for shift in [16, 8, 0] {
                w.u8((color >> shift) as u8);
            }
        }
        match v.target {
            WaypointTarget::Empty => w.varint(0),
            WaypointTarget::Block([x, y, z]) => {
                w.varint(1);
                w.varint(x);
                w.varint(y);
                w.varint(z);
            }
            WaypointTarget::Chunk([x, z]) => {
                w.varint(2);
                w.varint(x);
                w.varint(z);
            }
            WaypointTarget::Azimuth(angle) => {
                w.varint(3);
                w.f32(angle);
            }
        }
    }
}

// Generated retained-allocation traversals; regenerate with protocol-retained-gen.
impl crate::retained::OwnedHeap for WaypointId {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Uuid(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Name(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
        }
    }
}
impl crate::retained::OwnedHeap for WaypointTarget {
    fn owned_heap_bytes(&self) -> usize {
        match self {
            Self::Empty => 0usize,
            Self::Block(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Chunk(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
            Self::Azimuth(field_0) => {
                0usize.saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(field_0))
            }
        }
    }
}
impl crate::retained::OwnedHeap for TrackedWaypoint {
    fn owned_heap_bytes(&self) -> usize {
        let Self {
            operation,
            id,
            style,
            color,
            target,
        } = self;
        0usize
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(operation))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(id))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(style))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(color))
            .saturating_add(crate::retained::OwnedHeap::owned_heap_bytes(target))
    }
}
// End generated retained traversals.
