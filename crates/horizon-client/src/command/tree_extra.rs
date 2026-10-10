//! Commands Horizon parses but does not run (multiplayer, profiling and
//! server-only features): `/dialog`, `/kick`, `/posteffect`, `/fetchprofile`,
//! `/spectate`, `/publish`, `/unpublish`, `/waypoint`, `/debug`, `/jfr` and
//! the `/test` root.
use super::{
    Dispatcher,
    args::ArgType,
    vanilla::{ENTITY, GAMEMASTERS, PLAYER, PLAYERS},
};

const ADMINS: u8 = 3;
const OWNERS: u8 = 4;

pub fn register(d: &mut Dispatcher, root: usize) {
    // DialogCommand
    let dialog = d.literal(root, "dialog");
    d.requires(dialog, GAMEMASTERS);
    let show = d.literal(dialog, "show");
    let t = d.argument(show, "targets", PLAYERS);
    let dl = d.argument(t, "dialog", ArgType::ResourceOrId("minecraft:dialog"));
    d.executes(dl, "dialog.show");
    let clear = d.literal(dialog, "clear");
    let t = d.argument(clear, "targets", PLAYERS);
    d.executes(t, "dialog.clear");

    // KickCommand
    let kick = d.literal(root, "kick");
    d.requires(kick, ADMINS);
    let t = d.argument(kick, "targets", PLAYERS);
    d.executes(t, "kick");
    let reason = d.argument(t, "reason", ArgType::Message);
    d.executes(reason, "kick");

    // PostEffectCommand
    let pe = d.literal(root, "posteffect");
    d.requires(pe, GAMEMASTERS);
    let add = d.literal(pe, "add");
    let t = d.argument(add, "targets", PLAYERS);
    let id = d.argument(t, "posteffect", ArgType::Id("post_effects"));
    d.executes(id, "posteffect.add");
    let clear = d.literal(pe, "clear");
    let t = d.argument(clear, "targets", PLAYERS);
    d.executes(t, "posteffect.clear");
    let list = d.literal(pe, "list");
    let t = d.argument(list, "target", PLAYER);
    d.executes(t, "posteffect.list");
    let remove = d.literal(pe, "remove");
    let t = d.argument(remove, "targets", PLAYERS);
    let id = d.argument(t, "posteffect", ArgType::Id("post_effects"));
    d.executes(id, "posteffect.remove");

    // FetchProfileCommand
    let fp = d.literal(root, "fetchprofile");
    d.requires(fp, GAMEMASTERS);
    let name = d.literal(fp, "name");
    let n = d.argument(name, "name", ArgType::Greedy);
    d.executes(n, "fetchprofile");
    let id = d.literal(fp, "id");
    let u = d.argument(id, "id", ArgType::Uuid);
    d.executes(u, "fetchprofile");
    let entity = d.literal(fp, "entity");
    let e = d.argument(entity, "entity", ENTITY);
    d.executes(e, "fetchprofile");

    // SpectateCommand
    let spectate = d.literal(root, "spectate");
    d.requires(spectate, GAMEMASTERS);
    d.executes(spectate, "spectate");
    let target = d.argument(spectate, "target", ENTITY);
    d.executes(target, "spectate");
    let player = d.argument(target, "player", PLAYER);
    d.executes(player, "spectate");

    // PublishCommand / UnpublishCommand
    let publish = d.literal(root, "publish");
    d.requires(publish, OWNERS);
    d.executes(publish, "publish");
    let allow = d.argument(publish, "allowCommands", ArgType::Bool);
    d.executes(allow, "publish");
    let port = d.argument(allow, "port", ArgType::Integer { min: 0, max: 65535 });
    d.executes(port, "publish");
    let unpublish = d.literal(root, "unpublish");
    d.requires(unpublish, OWNERS);
    d.executes(unpublish, "unpublish");

    // WaypointCommand
    let wp = d.literal(root, "waypoint");
    d.requires(wp, GAMEMASTERS);
    let list = d.literal(wp, "list");
    d.executes(list, "waypoint.list");
    let modify = d.literal(wp, "modify");
    let w = d.argument(modify, "waypoint", ENTITY);
    let color = d.literal(w, "color");
    let c = d.argument(color, "color", ArgType::TeamColor);
    d.executes(c, "waypoint.modify");
    let hex = d.literal(color, "hex");
    let h = d.argument(hex, "color", ArgType::HexColor);
    d.executes(h, "waypoint.modify");
    let reset = d.literal(color, "reset");
    d.executes(reset, "waypoint.modify");
    let style = d.literal(w, "style");
    let reset = d.literal(style, "reset");
    d.executes(reset, "waypoint.modify");
    let set = d.literal(style, "set");
    let st = d.argument(set, "style", ArgType::Id("none"));
    d.executes(st, "waypoint.modify");

    // DebugCommand
    let debug = d.literal(root, "debug");
    d.requires(debug, ADMINS);
    for name in ["start", "stop"] {
        let l = d.literal(debug, name);
        d.executes(l, "debug");
    }
    let function = d.literal(debug, "function");
    d.requires(function, ADMINS);
    let n = d.argument(function, "name", ArgType::Function);
    d.suggests(n, super::Suggests::Functions);
    d.executes(n, "debug");

    // JfrCommand (registered when the JVM profiler is available)
    let jfr = d.literal(root, "jfr");
    d.requires(jfr, OWNERS);
    for name in ["start", "stop"] {
        let l = d.literal(jfr, name);
        d.executes(l, "jfr");
    }

    // TestCommand (game tests; only its root is modelled)
    let test = d.literal(root, "test");
    d.requires(test, GAMEMASTERS);
}
