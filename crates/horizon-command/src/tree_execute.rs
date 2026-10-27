//! The `/execute` tree (ExecuteCommand): modifiers redirect back to
//! `execute`, `run` to the root; conditions both execute (as a test) and
//! continue the chain; `store` redirects with a result callback.
use super::{
    Dispatcher,
    args::{ArgType, Resource},
    vanilla::{ENTITIES, GAMEMASTERS},
};

/// HeightmapTypeArgument: the types kept after world generation.
pub const HEIGHTMAPS: &[&str] = &[
    "world_surface",
    "ocean_floor",
    "motion_blocking",
    "motion_blocking_no_leaves",
];
pub const RELATIONS: [&str; 8] = [
    "owner",
    "leasher",
    "target",
    "attacker",
    "vehicle",
    "controller",
    "origin",
    "passengers",
];

/// A conditional's end: it runs as a test and continues the chain.
fn conditional(d: &mut Dispatcher, node: usize, ex: usize) {
    d.executes(node, "execute");
    d.redirect(node, ex);
}

pub fn register(d: &mut Dispatcher, root: usize) {
    let ex = d.literal(root, "execute");
    d.requires(ex, GAMEMASTERS);
    let run = d.literal(ex, "run");
    d.redirect(run, root);
    for name in ["as", "at"] {
        let l = d.literal(ex, name);
        let t = d.argument(l, "targets", ENTITIES);
        d.redirect(t, ex);
    }
    let positioned = d.literal(ex, "positioned");
    let pos = d.argument(positioned, "pos", ArgType::Vec3 { center: true });
    d.redirect(pos, ex);
    let as_ = d.literal(positioned, "as");
    let t = d.argument(as_, "targets", ENTITIES);
    d.redirect(t, ex);
    let over = d.literal(positioned, "over");
    let hm = d.argument(
        over,
        "heightmap",
        ArgType::StrEnum(HEIGHTMAPS, "argument.enum.invalid"),
    );
    d.redirect(hm, ex);
    let rotated = d.literal(ex, "rotated");
    let rot = d.argument(rotated, "rot", ArgType::Rotation);
    d.redirect(rot, ex);
    let as_ = d.literal(rotated, "as");
    let t = d.argument(as_, "targets", ENTITIES);
    d.redirect(t, ex);
    let facing = d.literal(ex, "facing");
    let pos = d.argument(facing, "pos", ArgType::Vec3 { center: true });
    d.redirect(pos, ex);
    let entity = d.literal(facing, "entity");
    let t = d.argument(entity, "targets", ENTITIES);
    let anchor = d.argument(t, "anchor", ArgType::Anchor);
    d.redirect(anchor, ex);
    let align = d.literal(ex, "align");
    let axes = d.argument(align, "axes", ArgType::Swizzle);
    d.redirect(axes, ex);
    let anchored = d.literal(ex, "anchored");
    let anchor = d.argument(anchored, "anchor", ArgType::Anchor);
    d.redirect(anchor, ex);
    let in_ = d.literal(ex, "in");
    let dim = d.argument(in_, "dimension", ArgType::Dimension);
    d.redirect(dim, ex);
    let summon = d.literal(ex, "summon");
    let e = d.argument(summon, "entity", ArgType::Resource(Resource::EntityType));
    d.suggests(e, super::Suggests::SummonableEntities);
    d.redirect(e, ex);
    let on = d.literal(ex, "on");
    for r in RELATIONS {
        let l = d.literal(on, r);
        d.redirect(l, ex);
    }
    store(d, ex);
    for mode in ["if", "unless"] {
        let m = d.literal(ex, mode);
        conditionals(d, m, ex);
    }
}

fn store(d: &mut Dispatcher, ex: usize) {
    let store = d.literal(ex, "store");
    for kind in ["result", "success"] {
        let k = d.literal(store, kind);
        let score = d.literal(k, "score");
        let targets = d.argument(score, "targets", ArgType::ScoreHolder { multiple: true });
        let objective = d.argument(targets, "objective", ArgType::Objective);
        d.redirect(objective, ex);
        let bossbar = d.literal(k, "bossbar");
        let id = d.argument(bossbar, "id", ArgType::Id("bossbars"));
        for what in ["value", "max"] {
            let w = d.literal(id, what);
            d.redirect(w, ex);
        }
        for provider in ["entity", "block", "storage"] {
            let p = d.literal(k, provider);
            let a = match provider {
                "entity" => d.argument(
                    p,
                    "target",
                    ArgType::Entity {
                        single: true,
                        players: false,
                    },
                ),
                "block" => d.argument(p, "targetPos", ArgType::BlockPos),
                _ => d.argument(p, "target", ArgType::Id("storages")),
            };
            let path = d.argument(a, "path", ArgType::NbtPath);
            for ty in ["int", "float", "short", "long", "double", "byte"] {
                let t = d.literal(path, ty);
                let scale = d.argument(
                    t,
                    "scale",
                    ArgType::Double {
                        min: f64::MIN,
                        max: f64::MAX,
                    },
                );
                d.redirect(scale, ex);
            }
        }
    }
}

fn conditionals(d: &mut Dispatcher, m: usize, ex: usize) {
    let block = d.literal(m, "block");
    let pos = d.argument(block, "pos", ArgType::BlockPos);
    let predicate = d.argument(pos, "block", ArgType::BlockPredicate);
    conditional(d, predicate, ex);

    let biome = d.literal(m, "biome");
    let pos = d.argument(biome, "pos", ArgType::BlockPos);
    let b = d.argument(pos, "biome", ArgType::ResOrTag("minecraft:worldgen/biome"));
    conditional(d, b, ex);

    let loaded = d.literal(m, "loaded");
    let pos = d.argument(loaded, "pos", ArgType::BlockPos);
    conditional(d, pos, ex);

    let dimension = d.literal(m, "dimension");
    let dim = d.argument(dimension, "dimension", ArgType::Dimension);
    conditional(d, dim, ex);

    let score = d.literal(m, "score");
    let target = d.argument(score, "target", ArgType::ScoreHolder { multiple: false });
    let objective = d.argument(target, "targetObjective", ArgType::Objective);
    for op in ["=", "<", "<=", ">", ">="] {
        let o = d.literal(objective, op);
        let source = d.argument(o, "source", ArgType::ScoreHolder { multiple: false });
        let so = d.argument(source, "sourceObjective", ArgType::Objective);
        conditional(d, so, ex);
    }
    let matches = d.literal(objective, "matches");
    let range = d.argument(matches, "range", ArgType::IntRange);
    conditional(d, range, ex);

    let blocks = d.literal(m, "blocks");
    let start = d.argument(blocks, "start", ArgType::BlockPos);
    let end = d.argument(start, "end", ArgType::BlockPos);
    let dest = d.argument(end, "destination", ArgType::BlockPos);
    for mode in ["all", "masked"] {
        let l = d.literal(dest, mode);
        conditional(d, l, ex);
    }

    let entity = d.literal(m, "entity");
    let targets = d.argument(entity, "entities", ENTITIES);
    conditional(d, targets, ex);

    let predicate = d.literal(m, "predicate");
    let p = d.argument(
        predicate,
        "predicate",
        ArgType::ResourceOrId("minecraft:predicate"),
    );
    conditional(d, p, ex);

    let stopwatch = d.literal(m, "stopwatch");
    let id = d.argument(stopwatch, "id", ArgType::Id("stopwatches"));
    let range = d.argument(id, "range", ArgType::FloatRange);
    conditional(d, range, ex);

    let function = d.literal(m, "function");
    let name = d.argument(function, "name", ArgType::Function);
    d.suggests(name, super::Suggests::Functions);
    // ExecuteIfFunctionCustomModifier: a fork only (not a test command).
    d.redirect(name, ex);

    let data = d.literal(m, "data");
    for provider in ["entity", "block", "storage"] {
        let p = d.literal(data, provider);
        let a = match provider {
            "entity" => d.argument(
                p,
                "source",
                ArgType::Entity {
                    single: true,
                    players: false,
                },
            ),
            "block" => d.argument(p, "sourcePos", ArgType::BlockPos),
            _ => d.argument(p, "source", ArgType::Id("storages")),
        };
        let path = d.argument(a, "path", ArgType::NbtPath);
        conditional(d, path, ex);
    }

    // items / slots (entity|block) <slots> [<predicate>]
    for kind in ["items", "slots"] {
        let k = d.literal(m, kind);
        for provider in ["entity", "block"] {
            let p = d.literal(k, provider);
            let a = if provider == "entity" {
                d.argument(
                    p,
                    "source",
                    ArgType::Entity {
                        single: false,
                        players: false,
                    },
                )
            } else {
                d.argument(p, "sourcePos", ArgType::BlockPos)
            };
            let slots = d.argument(a, "slots", ArgType::SlotSource);
            if kind == "items" {
                let item = d.argument(slots, "item_predicate", ArgType::ItemPredicate);
                conditional(d, item, ex);
            } else {
                conditional(d, slots, ex);
            }
        }
    }
}
