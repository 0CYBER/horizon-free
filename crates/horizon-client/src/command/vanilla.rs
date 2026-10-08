//! The vanilla command tree (Commands.java registrations) for the commands
//! Horizon implements. Leaves carry command ids that the game executes;
//! mode keywords are read back from the parsed literals.
use super::{
    Dispatcher,
    args::{ArgType, GameMode, Resource, integer},
};
use crate::gamerules::{RULES, Value};

/// Permission levels (Commands.LEVEL_*).
pub const ALL: u8 = 0;
pub const GAMEMASTERS: u8 = 2;

const PLAYERS: ArgType = ArgType::Entity { single: false, players: true };
const PLAYER: ArgType = ArgType::Entity { single: true, players: true };
const ENTITIES: ArgType = ArgType::Entity { single: false, players: false };
const ENTITY: ArgType = ArgType::Entity { single: true, players: false };

pub const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

pub fn build() -> Dispatcher {
    let mut d = Dispatcher::new();
    let root = 0;

    // help [<command>]
    let help = d.literal(root, "help");
    d.executes(help, "help");
    let c = d.argument(help, "command", ArgType::Greedy);
    d.executes(c, "help");

    // gamemode <gamemode> [<target>]
    let gm = d.literal(root, "gamemode");
    d.requires(gm, GAMEMASTERS);
    let mode = d.argument(gm, "gamemode", ArgType::GameMode);
    d.executes(mode, "gamemode");
    let t = d.argument(mode, "target", PLAYERS);
    d.executes(t, "gamemode");

    let dgm = d.literal(root, "defaultgamemode");
    d.requires(dgm, GAMEMASTERS);
    let mode = d.argument(dgm, "gamemode", ArgType::GameMode);
    d.executes(mode, "defaultgamemode");

    // give <targets> <item> [<count>]
    let give = d.literal(root, "give");
    d.requires(give, GAMEMASTERS);
    let targets = d.argument(give, "targets", PLAYERS);
    let item = d.argument(targets, "item", ArgType::Item);
    d.executes(item, "give");
    let count = d.argument(item, "count", integer(1));
    d.executes(count, "give");

    // teleport / tp
    let tp = d.literal(root, "teleport");
    d.requires(tp, GAMEMASTERS);
    let loc = d.argument(tp, "location", ArgType::Vec3 { center: true });
    d.executes(loc, "teleport.location");
    let dest = d.argument(tp, "destination", ENTITY);
    d.executes(dest, "teleport.destination");
    let targets = d.argument(tp, "targets", ENTITIES);
    let loc = d.argument(targets, "location", ArgType::Vec3 { center: true });
    d.executes(loc, "teleport.targets.location");
    let rot = d.argument(loc, "rotation", ArgType::Rotation);
    d.executes(rot, "teleport.targets.location");
    let facing = d.literal(loc, "facing");
    let fe = d.literal(facing, "entity");
    let fent = d.argument(fe, "facingEntity", ENTITY);
    d.executes(fent, "teleport.targets.location");
    let anchor = d.argument(fent, "facingAnchor", ArgType::Anchor);
    d.executes(anchor, "teleport.targets.location");
    let floc = d.argument(facing, "facingLocation", ArgType::Vec3 { center: true });
    d.executes(floc, "teleport.targets.location");
    let dest = d.argument(targets, "destination", ENTITY);
    d.executes(dest, "teleport.targets.destination");
    let alias = d.literal(root, "tp");
    d.requires(alias, GAMEMASTERS);
    d.redirect(alias, tp);

    // time: clock nodes for the dimension's default clock, plus `of <clock>`.
    let time = d.literal(root, "time");
    d.requires(time, GAMEMASTERS);
    clock_nodes(&mut d, time);
    let query = d.literal(time, "query");
    let gt = d.literal(query, "gametime");
    d.executes(gt, "time.query.gametime");
    let of = d.literal(time, "of");
    let clock = d.argument(of, "clock", ArgType::Resource(Resource::Clock));
    clock_nodes(&mut d, clock);

    // weather (clear|rain|thunder) [<duration>]
    let weather = d.literal(root, "weather");
    d.requires(weather, GAMEMASTERS);
    for kind in ["clear", "rain", "thunder"] {
        let k = d.literal(weather, kind);
        d.executes(k, "weather");
        let dur = d.argument(k, "duration", ArgType::Time { min: 1 });
        d.executes(dur, "weather");
    }

    // kill [<targets>]
    let kill = d.literal(root, "kill");
    d.requires(kill, GAMEMASTERS);
    d.executes(kill, "kill");
    let t = d.argument(kill, "targets", ENTITIES);
    d.executes(t, "kill");

    // summon <entity> [<pos>] [<nbt>]
    let summon = d.literal(root, "summon");
    d.requires(summon, GAMEMASTERS);
    let e = d.argument(summon, "entity", ArgType::Resource(Resource::EntityType));
    d.executes(e, "summon");
    let pos = d.argument(e, "pos", ArgType::Vec3 { center: true });
    d.executes(pos, "summon");
    let nbt = d.argument(pos, "nbt", ArgType::Nbt);
    d.executes(nbt, "summon");

    // setblock <pos> <block> [destroy|keep|replace|strict]
    let sb = d.literal(root, "setblock");
    d.requires(sb, GAMEMASTERS);
    let pos = d.argument(sb, "pos", ArgType::BlockPos);
    let block = d.argument(pos, "block", ArgType::BlockState);
    d.executes(block, "setblock");
    for m in ["destroy", "keep", "replace", "strict"] {
        let l = d.literal(block, m);
        d.executes(l, "setblock");
    }

    // fill <from> <to> <block> [modes]
    let fill = d.literal(root, "fill");
    d.requires(fill, GAMEMASTERS);
    let from = d.argument(fill, "from", ArgType::BlockPos);
    let to = d.argument(from, "to", ArgType::BlockPos);
    let block = d.argument(to, "block", ArgType::BlockState);
    fill_modes(&mut d, block);
    let replace = d.literal(block, "replace");
    d.executes(replace, "fill");
    let filter = d.argument(replace, "filter", ArgType::BlockPredicate);
    fill_modes(&mut d, filter);
    let keep = d.literal(block, "keep");
    d.executes(keep, "fill");

    // clone [from <dim>] <begin> <end> [to <dim>] <destination> [strict] [mask] [mode]
    let clone = d.literal(root, "clone");
    d.requires(clone, GAMEMASTERS);
    clone_begin(&mut d, clone);
    let from = d.literal(clone, "from");
    let dim = d.argument(from, "sourceDimension", ArgType::Dimension);
    clone_begin(&mut d, dim);

    // experience / xp
    let xp = d.literal(root, "experience");
    d.requires(xp, GAMEMASTERS);
    for (op, min) in [("add", i32::MIN), ("set", 0)] {
        let o = d.literal(xp, op);
        let t = d.argument(o, "target", PLAYERS);
        let amount = d.argument(t, "amount", integer(min));
        d.executes(amount, "experience");
        for unit in ["points", "levels"] {
            let u = d.literal(amount, unit);
            d.executes(u, "experience");
        }
    }
    let q = d.literal(xp, "query");
    let t = d.argument(q, "target", PLAYER);
    for unit in ["points", "levels"] {
        let u = d.literal(t, unit);
        d.executes(u, "experience");
    }
    let alias = d.literal(root, "xp");
    d.requires(alias, GAMEMASTERS);
    d.redirect(alias, xp);

    // gamerule <rule> [<value>] (short and namespaced names)
    let gr = d.literal(root, "gamerule");
    d.requires(gr, GAMEMASTERS);
    for rule in RULES {
        for name in [rule.name.to_owned(), format!("minecraft:{}", rule.name)] {
            let r = d.literal(gr, &name);
            d.executes(r, "gamerule");
            let ty = match rule.default {
                Value::Bool(_) => ArgType::Bool,
                Value::Int(_) => ArgType::Integer { min: rule.min, max: rule.max },
            };
            let v = d.argument(r, "value", ty);
            d.executes(v, "gamerule");
        }
    }

    // difficulty [<difficulty>]
    let diff = d.literal(root, "difficulty");
    d.executes(diff, "difficulty");
    for name in DIFFICULTIES {
        let l = d.literal(diff, name);
        d.requires(l, GAMEMASTERS);
        d.executes(l, "difficulty");
    }

    let seed = d.literal(root, "seed");
    d.executes(seed, "seed");

    // spawnpoint [<targets>] [<pos>] [<rotation>]
    let sp = d.literal(root, "spawnpoint");
    d.requires(sp, GAMEMASTERS);
    d.executes(sp, "spawnpoint");
    let t = d.argument(sp, "targets", PLAYERS);
    d.executes(t, "spawnpoint");
    let pos = d.argument(t, "pos", ArgType::BlockPos);
    d.executes(pos, "spawnpoint");
    let rot = d.argument(pos, "rotation", ArgType::Rotation);
    d.executes(rot, "spawnpoint");

    // setworldspawn [<pos>] [<rotation>]
    let ws = d.literal(root, "setworldspawn");
    d.requires(ws, GAMEMASTERS);
    d.executes(ws, "setworldspawn");
    let pos = d.argument(ws, "pos", ArgType::BlockPos);
    d.executes(pos, "setworldspawn");
    let rot = d.argument(pos, "rotation", ArgType::Rotation);
    d.executes(rot, "setworldspawn");

    // clear [<targets>] [<item>] [<maxCount>]
    let clear = d.literal(root, "clear");
    d.requires(clear, GAMEMASTERS);
    d.executes(clear, "clear");
    let t = d.argument(clear, "targets", PLAYERS);
    d.executes(t, "clear");
    let item = d.argument(t, "item", ArgType::ItemPredicate);
    d.executes(item, "clear");
    let max = d.argument(item, "maxCount", integer(0));
    d.executes(max, "clear");

    // locate (structure|biome) <...>
    let locate = d.literal(root, "locate");
    d.requires(locate, GAMEMASTERS);
    let s = d.literal(locate, "structure");
    let a = d.argument(s, "structure", ArgType::Resource(Resource::Structure));
    d.executes(a, "locate.structure");
    let b = d.literal(locate, "biome");
    let a = d.argument(b, "biome", ArgType::Resource(Resource::Biome));
    d.executes(a, "locate.biome");

    // say / me / msg / list
    let say = d.literal(root, "say");
    d.requires(say, GAMEMASTERS);
    let m = d.argument(say, "message", ArgType::Message);
    d.executes(m, "say");
    let me = d.literal(root, "me");
    let m = d.argument(me, "action", ArgType::Message);
    d.executes(m, "me");
    let msg = d.literal(root, "msg");
    let t = d.argument(msg, "targets", PLAYERS);
    let m = d.argument(t, "message", ArgType::Message);
    d.executes(m, "msg");
    for alias in ["tell", "w"] {
        let a = d.literal(root, alias);
        d.redirect(a, msg);
    }
    let list = d.literal(root, "list");
    d.executes(list, "list");
    let uuids = d.literal(list, "uuids");
    d.executes(uuids, "list");

    d
}

/// TimeCommand.addClockNodes.
fn clock_nodes(d: &mut Dispatcher, node: usize) {
    let set = d.literal(node, "set");
    let t = d.argument(set, "time", ArgType::Time { min: 0 });
    d.executes(t, "time.set");
    let marker = d.argument(set, "timemarker", ArgType::Id("time_markers"));
    d.executes(marker, "time.set.marker");
    let add = d.literal(node, "add");
    let t = d.argument(add, "time", ArgType::Time { min: i32::MIN });
    d.executes(t, "time.add");
    let pause = d.literal(node, "pause");
    d.executes(pause, "time.pause");
    let resume = d.literal(node, "resume");
    d.executes(resume, "time.resume");
    let rate = d.literal(node, "rate");
    let r = d.argument(rate, "rate", ArgType::Float { min: 1.0e-5, max: 1000.0 });
    d.executes(r, "time.rate");
    let query = d.literal(node, "query");
    let time = d.literal(query, "time");
    d.executes(time, "time.query.time");
    let tl = d.argument(query, "timeline", ArgType::Resource(Resource::Timeline));
    d.executes(tl, "time.query.timeline");
    let rep = d.literal(tl, "repetition");
    d.executes(rep, "time.query.repetition");
}
/// FillCommand.wrapWithMode.
fn fill_modes(d: &mut Dispatcher, node: usize) {
    d.executes(node, "fill");
    for m in ["outline", "hollow", "destroy", "strict"] {
        let l = d.literal(node, m);
        d.executes(l, "fill");
    }
}
fn clone_begin(d: &mut Dispatcher, node: usize) {
    let begin = d.argument(node, "begin", ArgType::BlockPos);
    let end = d.argument(begin, "end", ArgType::BlockPos);
    clone_destination(d, end);
    let to = d.literal(end, "to");
    let dim = d.argument(to, "targetDimension", ArgType::Dimension);
    clone_destination(d, dim);
}
fn clone_destination(d: &mut Dispatcher, node: usize) {
    let dest = d.argument(node, "destination", ArgType::BlockPos);
    clone_mask(d, dest);
    let strict = d.literal(dest, "strict");
    clone_mask(d, strict);
}
fn clone_mask(d: &mut Dispatcher, node: usize) {
    d.executes(node, "clone");
    for m in ["replace", "masked"] {
        let l = d.literal(node, m);
        clone_mode(d, l);
    }
    let filtered = d.literal(node, "filtered");
    let filter = d.argument(filtered, "filter", ArgType::BlockPredicate);
    clone_mode(d, filter);
}
fn clone_mode(d: &mut Dispatcher, node: usize) {
    d.executes(node, "clone");
    for m in ["force", "move", "normal"] {
        let l = d.literal(node, m);
        d.executes(l, "clone");
    }
}
/// Game mode names for selector `gamemode=` and the argument.
pub fn game_modes() -> [&'static str; 4] {
    GameMode::ALL.map(GameMode::name)
}
/// The shared vanilla tree.
pub fn dispatcher() -> &'static Dispatcher {
    static TREE: std::sync::OnceLock<Dispatcher> = std::sync::OnceLock::new();
    TREE.get_or_init(build)
}
#[allow(unused)]
const _: (u8, u8) = (ALL, GAMEMASTERS);
