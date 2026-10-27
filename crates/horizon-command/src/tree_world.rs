//! Command trees that act on entities and the world: `/recipe`, `/damage`,
//! `/rotate`, `/swing`, `/playsound`, `/stopsound`, `/particle`,
//! `/attribute`, `/ride`, `/spreadplayers`, `/loot`, `/forceload`,
//! `/fillbiome`, `/worldborder`.
use super::{
    Dispatcher,
    args::{ArgType, integer},
    vanilla::{ENTITIES, ENTITY, GAMEMASTERS, PLAYERS},
};
use horizon_client_core::sound_source::SOUND_SOURCES;

fn float(min: f32) -> ArgType {
    ArgType::Float { min, max: f32::MAX }
}
fn double() -> ArgType {
    ArgType::Double {
        min: f64::MIN,
        max: f64::MAX,
    }
}

pub fn register(d: &mut Dispatcher, root: usize) {
    recipe(d, root);
    damage(d, root);
    rotate_swing(d, root);
    sounds(d, root);
    particle(d, root);
    attribute(d, root);

    // ride <target> (mount <vehicle> | dismount)
    let ride = d.literal(root, "ride");
    d.requires(ride, GAMEMASTERS);
    let target = d.argument(ride, "target", ENTITY);
    let mount = d.literal(target, "mount");
    let vehicle = d.argument(mount, "vehicle", ENTITY);
    d.executes(vehicle, "ride.mount");
    let dismount = d.literal(target, "dismount");
    d.executes(dismount, "ride.dismount");

    // spreadplayers <center> <spreadDistance> <maxRange> [under <maxHeight>] <respectTeams> <targets>
    let spread = d.literal(root, "spreadplayers");
    d.requires(spread, GAMEMASTERS);
    let center = d.argument(spread, "center", ArgType::Vec2);
    let dist = d.argument(center, "spreadDistance", float(0.0));
    let range = d.argument(dist, "maxRange", float(1.0));
    let respect = d.argument(range, "respectTeams", ArgType::Bool);
    let targets = d.argument(respect, "targets", ENTITIES);
    d.executes(targets, "spreadplayers");
    let under = d.literal(range, "under");
    let height = d.argument(under, "maxHeight", integer(i32::MIN));
    let respect = d.argument(height, "respectTeams", ArgType::Bool);
    let targets = d.argument(respect, "targets", ENTITIES);
    d.executes(targets, "spreadplayers");

    // place template <template> [<pos> [<rotation> [<mirror> [<integrity> [<seed> [strict]]]]]]
    let place = d.literal(root, "place");
    d.requires(place, GAMEMASTERS);
    let feature = d.literal(place, "feature");
    let f = d.argument(
        feature,
        "feature",
        ArgType::ResourceOrId("minecraft:worldgen/feature"),
    );
    d.executes(f, "place.feature");
    let pos = d.argument(f, "pos", ArgType::BlockPos);
    d.executes(pos, "place.feature");
    let jigsaw = d.literal(place, "jigsaw");
    let pool = d.argument(
        jigsaw,
        "pool",
        ArgType::ResKey("minecraft:worldgen/template_pool"),
    );
    let target = d.argument(pool, "target", ArgType::Id("none"));
    let depth = d.argument(target, "max_depth", ArgType::Integer { min: 1, max: 20 });
    d.executes(depth, "place.jigsaw");
    let position = d.argument(depth, "position", ArgType::BlockPos);
    d.executes(position, "place.jigsaw");
    let structure = d.literal(place, "structure");
    let st = d.argument(
        structure,
        "structure",
        ArgType::ResKey("minecraft:worldgen/structure"),
    );
    d.executes(st, "place.structure");
    let pos = d.argument(st, "pos", ArgType::BlockPos);
    d.executes(pos, "place.structure");
    let template = d.literal(place, "template");
    let id = d.argument(template, "template", ArgType::Id("templates"));
    d.executes(id, "place");
    let pos = d.argument(id, "pos", ArgType::BlockPos);
    d.executes(pos, "place");
    let rotation = d.argument(
        pos,
        "rotation",
        ArgType::StrEnum(
            &["none", "clockwise_90", "180", "counterclockwise_90"],
            "argument.enum.invalid",
        ),
    );
    d.executes(rotation, "place");
    let mirror = d.argument(
        rotation,
        "mirror",
        ArgType::StrEnum(
            &["none", "left_right", "front_back"],
            "argument.enum.invalid",
        ),
    );
    d.executes(mirror, "place");
    let integrity = d.argument(mirror, "integrity", ArgType::Float { min: 0.0, max: 1.0 });
    d.executes(integrity, "place");
    let seed = d.argument(integrity, "seed", integer(i32::MIN));
    d.executes(seed, "place");
    let strict = d.literal(seed, "strict");
    d.executes(strict, "place");

    forceload(d, root);
    fillbiome(d, root);
    worldborder(d, root);
    loot(d, root);
}

fn recipe(d: &mut Dispatcher, root: usize) {
    let recipe = d.literal(root, "recipe");
    d.requires(recipe, GAMEMASTERS);
    for (word, id) in [("give", "recipe.give"), ("take", "recipe.take")] {
        let w = d.literal(recipe, word);
        let t = d.argument(w, "targets", PLAYERS);
        let r = d.argument(t, "recipe", ArgType::ResKey("minecraft:recipe"));
        d.executes(r, id);
        let all = d.literal(t, "*");
        d.executes(all, id);
    }
}

fn damage(d: &mut Dispatcher, root: usize) {
    let damage = d.literal(root, "damage");
    d.requires(damage, GAMEMASTERS);
    let target = d.argument(damage, "target", ENTITY);
    let amount = d.argument(target, "amount", float(0.0));
    d.executes(amount, "damage");
    let ty = d.argument(amount, "damageType", ArgType::Res("minecraft:damage_type"));
    d.executes(ty, "damage");
    let at = d.literal(ty, "at");
    let loc = d.argument(at, "location", ArgType::Vec3 { center: true });
    d.executes(loc, "damage");
    let by = d.literal(ty, "by");
    let entity = d.argument(by, "entity", ENTITY);
    d.executes(entity, "damage");
    let from = d.literal(entity, "from");
    let cause = d.argument(from, "cause", ENTITY);
    d.executes(cause, "damage");
}

fn rotate_swing(d: &mut Dispatcher, root: usize) {
    let rotate = d.literal(root, "rotate");
    d.requires(rotate, GAMEMASTERS);
    let target = d.argument(rotate, "target", ENTITY);
    let rot = d.argument(target, "rotation", ArgType::Rotation);
    d.executes(rot, "rotate");
    let facing = d.literal(target, "facing");
    let entity = d.literal(facing, "entity");
    let fe = d.argument(entity, "facingEntity", ENTITY);
    d.executes(fe, "rotate");
    let anchor = d.argument(fe, "facingAnchor", ArgType::Anchor);
    d.executes(anchor, "rotate");
    let loc = d.argument(facing, "facingLocation", ArgType::Vec3 { center: true });
    d.executes(loc, "rotate");

    let swing = d.literal(root, "swing");
    d.requires(swing, GAMEMASTERS);
    d.executes(swing, "swing");
    let targets = d.argument(swing, "targets", ENTITIES);
    d.executes(targets, "swing");
    for hand in ["mainhand", "offhand"] {
        let h = d.literal(targets, hand);
        d.executes(h, "swing");
        let animation = d.argument(
            h,
            "animation",
            ArgType::StrEnum(
                &["none", "whack", "stab"],
                "argument.swing_animation.invalid",
            ),
        );
        d.executes(animation, "swing");
        let duration = d.argument(animation, "duration", ArgType::Time { min: 1 });
        d.executes(duration, "swing");
    }
}

fn sounds(d: &mut Dispatcher, root: usize) {
    let play = d.literal(root, "playsound");
    d.requires(play, GAMEMASTERS);
    let sound = d.argument(play, "sound", ArgType::Id("sounds"));
    d.executes(sound, "playsound");
    let stop = d.literal(root, "stopsound");
    d.requires(stop, GAMEMASTERS);
    let stop_targets = d.argument(stop, "targets", PLAYERS);
    d.executes(stop_targets, "stopsound");
    let any = d.literal(stop_targets, "*");
    let any_sound = d.argument(any, "sound", ArgType::Id("sounds"));
    d.executes(any_sound, "stopsound");
    for source in SOUND_SOURCES {
        let s = d.literal(sound, source);
        d.executes(s, "playsound");
        let targets = d.argument(s, "targets", PLAYERS);
        d.executes(targets, "playsound");
        let pos = d.argument(targets, "pos", ArgType::Vec3 { center: true });
        d.executes(pos, "playsound");
        let volume = d.argument(pos, "volume", float(0.0));
        d.executes(volume, "playsound");
        let pitch = d.argument(volume, "pitch", ArgType::Float { min: 0.0, max: 2.0 });
        d.executes(pitch, "playsound");
        let min = d.argument(pitch, "minVolume", ArgType::Float { min: 0.0, max: 1.0 });
        d.executes(min, "playsound");

        let ls = d.literal(stop_targets, source);
        d.executes(ls, "stopsound");
        let lsound = d.argument(ls, "sound", ArgType::Id("sounds"));
        d.executes(lsound, "stopsound");
    }
}

fn particle(d: &mut Dispatcher, root: usize) {
    let particle = d.literal(root, "particle");
    d.requires(particle, GAMEMASTERS);
    let name = d.argument(particle, "name", ArgType::Particle);
    d.executes(name, "particle");
    let pos = d.argument(name, "pos", ArgType::Vec3 { center: true });
    d.executes(pos, "particle");
    let delta = d.argument(pos, "delta", ArgType::Vec3 { center: false });
    let speed = d.argument(delta, "speed", float(0.0));
    let count = d.argument(speed, "count", integer(0));
    d.executes(count, "particle");
    for mode in ["force", "normal"] {
        let m = d.literal(count, mode);
        d.executes(m, "particle");
        let viewers = d.argument(m, "viewers", PLAYERS);
        d.executes(viewers, "particle");
    }
}

fn attribute(d: &mut Dispatcher, root: usize) {
    let attribute = d.literal(root, "attribute");
    d.requires(attribute, GAMEMASTERS);
    let target = d.argument(attribute, "target", ENTITY);
    let attr = d.argument(target, "attribute", ArgType::Res("minecraft:attribute"));
    let get = d.literal(attr, "get");
    d.executes(get, "attribute.get");
    let scale = d.argument(get, "scale", double());
    d.executes(scale, "attribute.get");
    let base = d.literal(attr, "base");
    let set = d.literal(base, "set");
    let value = d.argument(set, "value", double());
    d.executes(value, "attribute.base.set");
    let bget = d.literal(base, "get");
    d.executes(bget, "attribute.base.get");
    let bscale = d.argument(bget, "scale", double());
    d.executes(bscale, "attribute.base.get");
    let reset = d.literal(base, "reset");
    d.executes(reset, "attribute.base.reset");
    let modifier = d.literal(attr, "modifier");
    let add = d.literal(modifier, "add");
    let id = d.argument(add, "id", ArgType::Id("attribute_modifiers"));
    let value = d.argument(id, "value", double());
    for op in ["add_value", "add_multiplied_base", "add_multiplied_total"] {
        let o = d.literal(value, op);
        d.executes(o, "attribute.modifier.add");
    }
    let remove = d.literal(modifier, "remove");
    let rid = d.argument(remove, "id", ArgType::Id("attribute_modifiers"));
    d.executes(rid, "attribute.modifier.remove");
    let value_l = d.literal(modifier, "value");
    let vget = d.literal(value_l, "get");
    let vid = d.argument(vget, "id", ArgType::Id("attribute_modifiers"));
    d.executes(vid, "attribute.modifier.get");
    let vscale = d.argument(vid, "scale", double());
    d.executes(vscale, "attribute.modifier.get");
}

fn forceload(d: &mut Dispatcher, root: usize) {
    let f = d.literal(root, "forceload");
    d.requires(f, GAMEMASTERS);
    for (word, id) in [("add", "forceload.add"), ("remove", "forceload.remove")] {
        let w = d.literal(f, word);
        let from = d.argument(w, "from", ArgType::ColumnPos);
        d.executes(from, id);
        let to = d.argument(from, "to", ArgType::ColumnPos);
        d.executes(to, id);
        if word == "remove" {
            let all = d.literal(w, "all");
            d.executes(all, "forceload.remove.all");
        }
    }
    let query = d.literal(f, "query");
    d.executes(query, "forceload.query");
    let pos = d.argument(query, "pos", ArgType::ColumnPos);
    d.executes(pos, "forceload.query");
}

fn fillbiome(d: &mut Dispatcher, root: usize) {
    let f = d.literal(root, "fillbiome");
    d.requires(f, GAMEMASTERS);
    let from = d.argument(f, "from", ArgType::BlockPos);
    let to = d.argument(from, "to", ArgType::BlockPos);
    let biome = d.argument(to, "biome", ArgType::Res("minecraft:worldgen/biome"));
    d.executes(biome, "fillbiome");
    let replace = d.literal(biome, "replace");
    let filter = d.argument(
        replace,
        "filter",
        ArgType::ResOrTag("minecraft:worldgen/biome"),
    );
    d.executes(filter, "fillbiome");
}

fn worldborder(d: &mut Dispatcher, root: usize) {
    let w = d.literal(root, "worldborder");
    d.requires(w, GAMEMASTERS);
    let dist = || ArgType::Double {
        min: -5.999_996_8e7,
        max: 5.999_996_8e7,
    };
    for word in ["add", "set"] {
        let l = d.literal(w, word);
        let distance = d.argument(l, "distance", dist());
        let id = if word == "add" {
            "worldborder.add"
        } else {
            "worldborder.set"
        };
        d.executes(distance, id);
        let time = d.argument(distance, "time", ArgType::Time { min: 0 });
        d.executes(time, id);
    }
    let center = d.literal(w, "center");
    let pos = d.argument(center, "pos", ArgType::Vec2);
    d.executes(pos, "worldborder.center");
    let damage = d.literal(w, "damage");
    let amount = d.literal(damage, "amount");
    let per = d.argument(amount, "damagePerBlock", float(0.0));
    d.executes(per, "worldborder.damage.amount");
    let buffer = d.literal(damage, "buffer");
    let b = d.argument(buffer, "distance", float(0.0));
    d.executes(b, "worldborder.damage.buffer");
    let get = d.literal(w, "get");
    d.executes(get, "worldborder.get");
    let warning = d.literal(w, "warning");
    let wd = d.literal(warning, "distance");
    let v = d.argument(wd, "distance", integer(0));
    d.executes(v, "worldborder.warning.distance");
    let wt = d.literal(warning, "time");
    let v = d.argument(wt, "time", ArgType::Time { min: 0 });
    d.executes(v, "worldborder.warning.time");
}

/// `loot (replace|insert|give|spawn) ... (fish|loot|kill|mine)`.
fn loot(d: &mut Dispatcher, root: usize) {
    let loot = d.literal(root, "loot");
    d.requires(loot, GAMEMASTERS);
    // Targets.
    let mut targets = Vec::new();
    let replace = d.literal(loot, "replace");
    for provider in ["entity", "block"] {
        let p = d.literal(replace, provider);
        let a = if provider == "entity" {
            d.argument(p, "entities", ENTITIES)
        } else {
            d.argument(p, "targetPos", ArgType::BlockPos)
        };
        let slot = d.argument(a, "slot", ArgType::Slot);
        targets.push(slot);
        let count = d.argument(slot, "count", integer(0));
        targets.push(count);
    }
    let insert = d.literal(loot, "insert");
    targets.push(d.argument(insert, "targetPos", ArgType::BlockPos));
    let give = d.literal(loot, "give");
    targets.push(d.argument(give, "players", PLAYERS));
    let spawn = d.literal(loot, "spawn");
    targets.push(d.argument(spawn, "targetPos", ArgType::Vec3 { center: true }));
    // Sources.
    for t in targets {
        let fish = d.literal(t, "fish");
        let table = d.argument(
            fish,
            "loot_table",
            ArgType::ResourceOrId("minecraft:loot_table"),
        );
        let pos = d.argument(table, "pos", ArgType::BlockPos);
        d.executes(pos, "loot.fish");
        for hand in ["mainhand", "offhand"] {
            let h = d.literal(pos, hand);
            d.executes(h, "loot.fish");
        }
        let tool = d.argument(pos, "tool", ArgType::Item);
        d.executes(tool, "loot.fish");
        let l = d.literal(t, "loot");
        let lt = d.argument(
            l,
            "loot_table",
            ArgType::ResourceOrId("minecraft:loot_table"),
        );
        d.executes(lt, "loot.loot");
        let kill = d.literal(t, "kill");
        let target = d.argument(kill, "target", ENTITY);
        d.executes(target, "loot.kill");
        let mine = d.literal(t, "mine");
        let pos = d.argument(mine, "pos", ArgType::BlockPos);
        d.executes(pos, "loot.mine");
        for hand in ["mainhand", "offhand"] {
            let h = d.literal(pos, hand);
            d.executes(h, "loot.mine");
        }
        let tool = d.argument(pos, "tool", ArgType::Item);
        d.executes(tool, "loot.mine");
    }
}
