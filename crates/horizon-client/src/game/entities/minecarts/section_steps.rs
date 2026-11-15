use super::*;

pub(super) fn tick(
    game: &mut Game<'_>,
    id: u32,
    i: usize,
    index: &mut CartIndex,
) -> (bool, Option<(Vec3, Vec3)>) {
    let loot = game.carts.update(i, |c| c.loot.take());
    let mut cart = game.carts[i].clone();
    let from = cart.position;
    cart.loot = loot;
    let alive = game.tick_cart(&mut cart, [0.0; 2], i, index);
    game.hopper_cart_tick(&mut cart, alive);
    let Some(i) = cart_at(&game.carts, id, i) else {
        return (false, None);
    };
    if !alive {
        return (false, None);
    }
    let due = std::mem::take(&mut cart.command_due);
    let to = cart.position;
    index.moved(i, cart.bounds());
    let stored = game.store_cart_back(i, cart);
    (due, stored.then_some((from, to)))
}

pub(super) fn tick_riding(
    game: &mut Game<'_>,
    id: u32,
    i: usize,
    intent: [f64; 2],
) -> Option<(usize, bool, Option<(Vec3, Vec3)>, Option<(Vec3, Vec3)>)> {
    let mut cart = game.carts[i].clone();
    let from = cart.position;
    let alive = game.tick_cart(&mut cart, intent, i, &mut CartIndex::default());
    // `MinecartHopper.tick`'s `tryConsumeItems` runs in the rider's turn too.
    game.hopper_cart_tick(&mut cart, alive);
    let mut player_step = None;
    if alive
        && game.riding == Some(cart.id)
        && let Some(player) = game.player.as_mut()
    {
        let player_from = player.position;
        game.previous_position = player.position;
        player.position = Vec3::new(cart.position.x, cart.position.y + SEAT, cart.position.z);
        player.velocity = Vec3::ZERO;
        player.fall_distance = 0.0;
        player_step = Some((player_from, player.position));
    }
    let Some(i) = game.carts.iter().position(|x| x.id == cart.id) else {
        if alive && game.riding == Some(id) && !game.persist.removed_vehicles.contains_key(&id) {
            game.riding = None;
        }
        return None;
    };
    if !alive {
        return Some((i, false, None, player_step));
    }
    let to = cart.position;
    let stored = game.store_cart_back(i, cart);
    Some((i, true, stored.then_some((from, to)), player_step))
}

pub(super) fn tell(game: &mut Game<'_>, id: u32, step: Option<(Vec3, Vec3)>) {
    if let Some((from, to)) = step {
        game.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
}

pub(super) fn tell_player(game: &mut Game<'_>, step: Option<(Vec3, Vec3)>) {
    if let Some((from, to)) = step
        && let Ok(id) = u32::try_from(game.own_entity_id())
    {
        game.entity_section_step(id, [from.x, from.y, from.z], [to.x, to.y, to.z]);
    }
}
