//! Pandas on the game side: the food they find on the ground and pick up
//! (`Mob.aiStep`, `Panda.pickUpItem`), the bamboo a player gives them
//! (`Panda.mobInteract`) and the start a sneeze gives the grown pandas about.
use crate::panda_level::PandaLevel;
use horizon_client::{
    entities::ItemEntity,
    items::ItemStack,
    join_seq::RestoreOrder,
    mobs::{Species, panda},
};
use horizon_core::geometry::Vec3;

/// A panda takes what lies at its feet when its mouth is free, and
/// looks for food within 6 and 8 blocks for `PandaSitGoal`.
pub fn tick_pandas<L: PandaLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    for i in 0..l.mobs().len() {
        let p = l.parts();
        let m = &p.mobs[i];
        if m.species() != Species::Panda || !m.alive() || !p.ticked.contains(&m.id) {
            continue;
        }
        let id = m.id;
        let wants = |lv: &L, e: &ItemEntity| {
            e.pickup_delay == 0 && lv.panda_eats_from_ground(&e.stack.item)
        };
        // Mob.aiStep: the items within the box grown by (1, 0, 1).
        let m = &l.mobs()[i];
        if m.can_pick_up_loot()
            && l.entity_grief(m.id, griefing)
            && m.extra.main_hand_id().is_none()
        {
            let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
            let taken = l
                .dropped()
                .iter()
                .position(|e| wants(&*l, e) && reach.intersects(e.bounds()));
            if let Some(j) = taken {
                let before = l.dropped()[j].clone();
                // `Panda.pickUpItem`'s `onItemPickup` comes first: the
                // advancement of whoever threw it.
                l.on_item_pickup(i, &before);
                let e = l.discard_item(j);
                l.take(id, e.clone());
                panda::hold(&mut l.parts().mobs[i], &e.stack.item, e.stack.count, true);
            }
        }
        let m = &l.mobs()[i];
        let area = |r: f64| m.bounds().inflate(Vec3::new(r, r, r));
        let (near, wide) = (area(6.0), area(8.0));
        let food_near = l
            .dropped()
            .iter()
            .any(|e| wants(&*l, e) && near.intersects(e.bounds()));
        let food_at = l
            .dropped()
            .iter()
            .find(|e| wants(&*l, e) && wide.intersects(e.bounds()))
            .map(|e| [e.position.x, e.position.y, e.position.z]);
        panda::set_food(&mut l.parts().mobs[i], food_near, food_at);
    }
}

/// `Panda.mobInteract` for a click with `item`; whether the click is taken.
pub fn interact_panda<L: PandaLevel + ?Sized>(l: &mut L, index: usize, item: &str) -> bool {
    let creative = l.is_creative();
    let m = &mut l.parts().mobs[index];
    if panda::scared(m) {
        return false;
    }
    if panda::get_up(m) {
        return true;
    }
    // A baby holding a golden_dandelion goes on to `AgeableMob.mobInteract`,
    // which `interact_mob` answered before this; anything else that is not
    // bamboo passes.
    if !m.kind.food.iter().any(|f| f == item) {
        return false;
    }
    if m.ai.target.is_some() {
        panda::got_bamboo(m);
    }
    let at = [m.position.x, m.position.y, m.position.z];
    let id = m.id;
    let mut dropped = None;
    let mut loved = false;
    if m.is_baby() {
        // Panda.mobInteract: a baby an age lock holds back passes.
        if !horizon_client::mobs::ai::can_age_up(m) {
            return false;
        }
        // ageUp((int) (-getAge() / 20 * 0.1F), true).
        let seconds = ((-m.extra.age / 20) as f32 * 0.1) as i32;
        horizon_client::mobs::ai::age_up(m, seconds, true);
    } else if m.extra.age == 0 && m.extra.in_love <= 0 {
        m.extra.in_love = 600;
        m.extra.love_player = true;
        loved = true;
    } else {
        if panda::is_sitting(m) || m.fluids.in_water {
            return false;
        }
        panda::try_to_sit(m);
        panda::start_eating(m);
        // What it held falls, unless the player has infinite materials.
        if let Some(held) = m.extra.main_hand_id().map(str::to_owned)
            && !creative
        {
            dropped = Some(ItemStack::new(&held, panda::held_count(m)));
        }
        panda::hold(m, item, 1, false);
    }
    if loved {
        l.panda_in_love(id);
    }
    l.consume_held();
    if let Some(stack) = dropped {
        l.spawn_item(stack, at, [0.0, 0.2, 0.0], 10);
    }
    true
}

/// `Panda.afterSneeze`: the grown pandas within 10 blocks of a cub jump.
pub fn panda_sneezed<L: PandaLevel + ?Sized>(l: &mut L, id: u32) {
    let Some(area) = l
        .mobs()
        .iter()
        .find(|m| m.id == id)
        .map(|m| m.bounds().inflate(Vec3::new(10.0, 10.0, 10.0)))
    else {
        return;
    };
    let mut mobs = l.parts().mobs.lift();
    l.after_mobs_lifted();
    let w = l.mob_world();
    for m in &mut mobs {
        if m.species() == Species::Panda
            && !m.is_baby()
            && m.on_ground
            && !m.fluids.in_water
            && panda::can_perform_action(m, panda::scared(m))
            && area.intersects(m.bounds())
        {
            // LivingEntity.jumpFromGround.
            m.needs_sync = true;
            let power = f64::from(m.jump_power(&w));
            if power > 1.0e-5 {
                m.velocity.y = power.max(m.velocity.y);
            }
        }
    }
    l.parts().mobs.restore(mobs, RestoreOrder::SurvivorsFirst);
}
