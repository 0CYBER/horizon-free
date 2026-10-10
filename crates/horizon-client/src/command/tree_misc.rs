//! Smaller command trees: `/bossbar`, `/function`, `/return`, `/schedule`,
//! `/random`, `/tick`, `/teammsg`, `/reload` and `/datapack`.
use super::{
    Dispatcher,
    args::ArgType,
    vanilla::{ENTITIES, GAMEMASTERS, PLAYERS},
};

fn int() -> ArgType {
    ArgType::Integer {
        min: i32::MIN,
        max: i32::MAX,
    }
}
fn int_min(min: i32) -> ArgType {
    ArgType::Integer { min, max: i32::MAX }
}
pub const ADMINS: u8 = 3;

pub fn register(d: &mut Dispatcher, root: usize) {
    item(d, root);
    bossbar(d, root);
    function(d, root);

    // return <value> | fail | run <command>
    let ret = d.literal(root, "return");
    d.requires(ret, GAMEMASTERS);
    let v = d.argument(ret, "value", int());
    d.executes(v, "return.value");
    let fail = d.literal(ret, "fail");
    d.executes(fail, "return.fail");
    let run = d.literal(ret, "run");
    d.redirect(run, root);

    // schedule function <function> <time> [append|replace] | clear <function>
    let schedule = d.literal(root, "schedule");
    d.requires(schedule, GAMEMASTERS);
    let f = d.literal(schedule, "function");
    let name = d.argument(f, "function", ArgType::Function);
    let time = d.argument(name, "time", ArgType::Time { min: 0 });
    d.executes(time, "schedule.function");
    for mode in ["append", "replace"] {
        let m = d.literal(time, mode);
        d.executes(m, "schedule.function");
    }
    let clear = d.literal(schedule, "clear");
    let id = d.argument(clear, "function", ArgType::Id("schedules"));
    d.executes(id, "schedule.clear");

    // random (value|roll) <range> [<sequence>] | reset (*|<sequence>) ...
    let random = d.literal(root, "random");
    for kind in ["value", "roll"] {
        let k = d.literal(random, kind);
        let range = d.argument(k, "range", ArgType::IntRange);
        let id = if kind == "value" {
            "random.value"
        } else {
            "random.roll"
        };
        d.executes(range, id);
        let seq = d.argument(range, "sequence", ArgType::Id("random_sequences"));
        d.requires(seq, GAMEMASTERS);
        d.executes(seq, id);
    }
    let reset = d.literal(random, "reset");
    d.requires(reset, GAMEMASTERS);
    let all = d.literal(reset, "*");
    d.executes(all, "random.reset");
    let seq = d.argument(reset, "sequence", ArgType::Id("random_sequences"));
    d.executes(seq, "random.reset");
    for parent in [all, seq] {
        let seed = d.argument(parent, "seed", int());
        d.executes(seed, "random.reset");
        let world = d.argument(seed, "includeWorldSeed", ArgType::Bool);
        d.executes(world, "random.reset");
        let seq_id = d.argument(world, "includeSequenceId", ArgType::Bool);
        d.executes(seq_id, "random.reset");
    }

    // tick query | rate <rate> | step [stop|<time>] | sprint (stop|<time>) | freeze | unfreeze
    let tick = d.literal(root, "tick");
    d.requires(tick, ADMINS);
    let q = d.literal(tick, "query");
    d.executes(q, "tick.query");
    let rate = d.literal(tick, "rate");
    let r = d.argument(
        rate,
        "rate",
        ArgType::Float {
            min: 1.0,
            max: 10000.0,
        },
    );
    d.executes(r, "tick.rate");
    let step = d.literal(tick, "step");
    d.executes(step, "tick.step");
    let stop = d.literal(step, "stop");
    d.executes(stop, "tick.step.stop");
    let t = d.argument(step, "time", ArgType::Time { min: 1 });
    d.executes(t, "tick.step");
    let sprint = d.literal(tick, "sprint");
    let stop = d.literal(sprint, "stop");
    d.executes(stop, "tick.sprint.stop");
    let t = d.argument(sprint, "time", ArgType::Time { min: 1 });
    d.executes(t, "tick.sprint");
    let unfreeze = d.literal(tick, "unfreeze");
    d.executes(unfreeze, "tick.unfreeze");
    let freeze = d.literal(tick, "freeze");
    d.executes(freeze, "tick.freeze");

    // teammsg / tm <message>
    let teammsg = d.literal(root, "teammsg");
    let m = d.argument(teammsg, "message", ArgType::Message);
    d.executes(m, "teammsg");
    let tm = d.literal(root, "tm");
    d.redirect(tm, teammsg);

    // reload
    let reload = d.literal(root, "reload");
    d.requires(reload, GAMEMASTERS);
    d.executes(reload, "reload");

    // datapack list [available|enabled] | enable|disable <name> ...
    let datapack = d.literal(root, "datapack");
    d.requires(datapack, GAMEMASTERS);
    let list = d.literal(datapack, "list");
    d.executes(list, "datapack.list");
    for w in ["available", "enabled"] {
        let l = d.literal(list, w);
        d.executes(l, "datapack.list");
    }
    let enable = d.literal(datapack, "enable");
    let name = d.argument(enable, "name", ArgType::Name("datapacks_disabled"));
    d.executes(name, "datapack.enable");
    for w in ["first", "last"] {
        let l = d.literal(name, w);
        d.executes(l, "datapack.enable");
    }
    for w in ["before", "after"] {
        let l = d.literal(name, w);
        let other = d.argument(l, "existing", ArgType::Name("datapacks_enabled"));
        d.executes(other, "datapack.enable");
    }
    let disable = d.literal(datapack, "disable");
    let name = d.argument(disable, "name", ArgType::Name("datapacks_enabled"));
    d.executes(name, "datapack.disable");
}

/// `item (replace|fill|override) (entity|block) <target> <slots> (with <item> [<count>] | from ...)`.
fn item(d: &mut Dispatcher, root: usize) {
    let item = d.literal(root, "item");
    d.requires(item, GAMEMASTERS);
    for mode in ["replace", "fill", "override"] {
        let m = d.literal(item, mode);
        for provider in ["entity", "block"] {
            let p = d.literal(m, provider);
            let target = if provider == "entity" {
                d.argument(p, "target", ENTITIES)
            } else {
                d.argument(p, "targetPos", ArgType::BlockPos)
            };
            let slots = d.argument(target, "slots", ArgType::Slots);
            let with = d.literal(slots, "with");
            let it = d.argument(with, "item", ArgType::Item);
            d.executes(it, "item");
            let count = d.argument(it, "count", ArgType::Integer { min: 1, max: 99 });
            d.executes(count, "item");
            let from = d.literal(slots, "from");
            for src in ["entity", "block"] {
                let s = d.literal(from, src);
                let source = if src == "entity" {
                    d.argument(s, "source", ENTITIES)
                } else {
                    d.argument(s, "sourcePos", ArgType::BlockPos)
                };
                let source_slots = d.argument(source, "sourceSlots", ArgType::Slots);
                d.executes(source_slots, "item");
            }
        }
    }
}

fn bossbar(d: &mut Dispatcher, root: usize) {
    let b = d.literal(root, "bossbar");
    d.requires(b, GAMEMASTERS);
    let add = d.literal(b, "add");
    let id = d.argument(add, "id", ArgType::Id("bossbars_new"));
    let name = d.argument(id, "name", ArgType::Component);
    d.executes(name, "bossbar.add");
    let remove = d.literal(b, "remove");
    let id = d.argument(remove, "id", ArgType::Id("bossbars"));
    d.executes(id, "bossbar.remove");
    let list = d.literal(b, "list");
    d.executes(list, "bossbar.list");
    let set = d.literal(b, "set");
    let id = d.argument(set, "id", ArgType::Id("bossbars"));
    let name_l = d.literal(id, "name");
    let name = d.argument(name_l, "name", ArgType::Component);
    d.executes(name, "bossbar.set.name");
    let color = d.literal(id, "color");
    for c in crate::bossbar::COLORS {
        let l = d.literal(color, c);
        d.executes(l, "bossbar.set.color");
    }
    let style = d.literal(id, "style");
    for s in crate::bossbar::STYLES {
        let l = d.literal(style, s);
        d.executes(l, "bossbar.set.style");
    }
    let value = d.literal(id, "value");
    let v = d.argument(value, "value", int_min(0));
    d.executes(v, "bossbar.set.value");
    let max = d.literal(id, "max");
    let v = d.argument(max, "max", int_min(1));
    d.executes(v, "bossbar.set.max");
    let visible = d.literal(id, "visible");
    let v = d.argument(visible, "visible", ArgType::Bool);
    d.executes(v, "bossbar.set.visible");
    let players = d.literal(id, "players");
    d.executes(players, "bossbar.set.players");
    let t = d.argument(players, "targets", PLAYERS);
    d.executes(t, "bossbar.set.players");
    let get = d.literal(b, "get");
    let id = d.argument(get, "id", ArgType::Id("bossbars"));
    for w in ["value", "max", "visible", "players"] {
        let l = d.literal(id, w);
        d.executes(l, "bossbar.get");
    }
}

fn function(d: &mut Dispatcher, root: usize) {
    let f = d.literal(root, "function");
    d.requires(f, GAMEMASTERS);
    let name = d.argument(f, "name", ArgType::Function);
    d.executes(name, "function");
    let args = d.argument(name, "arguments", ArgType::Nbt);
    d.executes(args, "function");
    let with = d.literal(name, "with");
    for provider in ["entity", "block", "storage"] {
        let p = d.literal(with, provider);
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
        d.executes(a, "function");
        let path = d.argument(a, "path", ArgType::NbtPath);
        d.executes(path, "function");
    }
}
