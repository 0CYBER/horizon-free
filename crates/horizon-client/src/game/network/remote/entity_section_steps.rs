use super::Game;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(super) fn remote_move_entity_and_file(
        &mut self,
        id: i32,
        path: Option<super::Path>,
        rotation: Option<(f32, f32)>,
        on_ground: bool,
    ) {
        self.move_entity(id, path, rotation, on_ground, super::MovePacket::Move);
        self.entity_section_moved_id(id as u32);
    }

    pub(super) fn remote_position_sync_and_file(
        &mut self,
        id: i32,
        path: super::Path,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    ) {
        self.move_entity(
            id,
            Some(path),
            Some((yaw, pitch)),
            on_ground,
            super::MovePacket::PositionSync,
        );
        self.entity_section_moved_id(id as u32);
    }

    pub(super) fn remote_move_vehicle_and_file(&mut self, position: [f64; 3], yaw: f32) {
        self.remote_move_vehicle(position, yaw);
        if let Some(mount) = self.driven_mount() {
            self.entity_section_moved_id(mount.id);
        }
    }

    pub(super) fn remote_entity_section_step(&mut self, id: u32, from: Vec3, to: Vec3) {
        self.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
}

pub(super) fn player_teleport(game: &mut Game<'_>, had_player: bool, from: Vec3, to: Vec3) {
    if had_player && let Ok(id) = u32::try_from(game.own_entity_id()) {
        game.remote_entity_section_step(id, from, to);
    }
}

pub(super) fn player_before(game: &Game<'_>) -> Option<Vec3> {
    game.player.as_ref().map(|player| player.position)
}

pub(super) fn player_after(game: &mut Game<'_>, from: Option<Vec3>) {
    if let (Some(from), Some(to), Ok(id)) = (
        from,
        game.player.as_ref().map(|player| player.position),
        u32::try_from(game.own_entity_id()),
    ) && from != to
    {
        game.remote_entity_section_step(id, from, to);
    }
}

pub(super) fn tick_items(
    game: &mut Game<'_>,
    entered: &mut Vec<horizon_client::entities::ItemSplash>,
) {
    let mut items = game.dropped.lift();
    let mut i = 0;
    while i < items.len() {
        let (_, discarded, _, _) =
            game.tick_item_with_block_contacts(&mut items[i], None, &[], true, &mut |splash| {
                entered.push(splash)
            });
        let from = items[i].previous;
        let to = items[i].position;
        if !discarded {
            let live = game.entity_section_step_visible(
                items[i].id,
                [from.x, from.y, from.z],
                [to.x, to.y, to.z],
            );
            if !live {
                items.remove(i);
                continue;
            }
        }
        if discarded {
            items.remove(i);
        } else {
            i += 1;
        }
    }
    game.put_back_dropped(items);
}
