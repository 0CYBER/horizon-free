use super::Game;
use horizon_client::projectiles::Projectile;
use horizon_core::geometry::Vec3;

pub(super) struct Steps {
    enabled: bool,
    seen: Vec<u32>,
}

pub(super) fn begin(game: &mut Game<'_>) -> Steps {
    Steps {
        enabled: game.entity_section_scope().is_some(),
        seen: std::mem::take(&mut game.native_lifecycle.entity_sections.projectile_ids),
    }
}

pub(super) fn record(
    game: &mut Game<'_>,
    steps: &mut Steps,
    projectile: Option<&Projectile>,
    from: Vec3,
    keep: bool,
) -> bool {
    if let Some(p) = projectile
        && p.position != from
        && steps.enabled
        && keep
    {
        steps.seen.push(p.id);
        return game.entity_section_step_visible(
            p.id,
            [from.x, from.y, from.z],
            [p.position.x, p.position.y, p.position.z],
        );
    }
    true
}

pub(super) fn finish(game: &mut Game<'_>, mut steps: Steps) {
    if steps.enabled {
        {
            let live = &mut game.native_lifecycle.entity_sections.projectile_live;
            live.clear();
            live.extend(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|projectile| projectile.id),
            );
        }
        for id in steps.seen.drain(..) {
            if !game
                .native_lifecycle
                .entity_sections
                .projectile_live
                .contains(&id)
            {
                game.entity_section_forget(id);
            }
        }
    } else {
        steps.seen.clear();
    }
    steps.seen.extend(std::mem::take(
        &mut game.native_lifecycle.entity_sections.projectile_ids,
    ));
    game.native_lifecycle.entity_sections.projectile_ids = steps.seen;
}

pub(super) fn teleport_player(game: &mut Game<'_>, to: Vec3) {
    game.stop_riding();
    let from = game.player.as_ref().map(|player| player.position);
    if let Some(player) = game.player.as_mut() {
        player.position = to;
        player.velocity = Vec3::ZERO;
        player.fall_distance = 0.0;
        player.impulse.reset();
        player.teleported = true;
        player.teleported_delta = [false; 3];
    }
    game.previous_position = to;
    if let (Some(from), Ok(id)) = (from, u32::try_from(game.own_entity_id())) {
        game.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
}
