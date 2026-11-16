//! `Hit`: the block a click aimed at, in the terms the client's targeting and
//! the packets that carry it use.
#[derive(Clone, Copy)]
pub(crate) struct Hit {
    pub(crate) pos: [i32; 3],
    pub(crate) face: [i32; 3],
    pub(crate) point: [f64; 3],
    /// `BlockHitResult.isInside`: the ray started inside the clicked block's
    /// outline, so `face` is the side the look comes from.
    pub(crate) inside: bool,
}

impl Hit {
    /// The hit as block-use hooks hear it (`UseOnContext`'s hit result).
    pub(super) fn native(&self) -> mod_api::BlockHit {
        mod_api::BlockHit::new(self.point, self.face, self.inside)
    }
    /// The block the clicked face leads to (`pos.relative(direction)`).
    pub(super) fn beside(&self) -> [i32; 3] {
        [
            self.pos[0] + self.face[0],
            self.pos[1] + self.face[1],
            self.pos[2] + self.face[2],
        ]
    }
    /// `VoxelShape.clip`'s hit, which already carries the inside flag.
    pub(super) fn from_block_hit(pos: [i32; 3], hit: &horizon_core::geometry::BlockHit) -> Self {
        Self {
            pos,
            face: hit.direction.normal(),
            point: [hit.location.x, hit.location.y, hit.location.z],
            inside: hit.inside,
        }
    }
    /// The block a guest's `ServerboundUseItemOnPacket` aimed at
    /// (`ServerGamePacketListenerImpl.handleUseItemOn`).
    pub(super) fn from_use_item_on(p: &horizon_protocol::packets::play::UseItemOn) -> Self {
        Self {
            pos: p.pos,
            face: direction(p.face),
            point: [
                f64::from(p.pos[0]) + f64::from(p.cursor[0]),
                f64::from(p.pos[1]) + f64::from(p.cursor[1]),
                f64::from(p.pos[2]) + f64::from(p.cursor[2]),
            ],
            inside: p.inside,
        }
    }
}

/// A packet's `Direction.get3DDataValue` back to its step (`Direction.STREAM_CODEC`).
pub(super) fn direction(face: i32) -> [i32; 3] {
    match face {
        0 => [0, -1, 0],
        1 => [0, 1, 0],
        2 => [0, 0, -1],
        3 => [0, 0, 1],
        4 => [-1, 0, 0],
        _ => [1, 0, 0],
    }
}
