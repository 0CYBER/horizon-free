//! The `/scoreboard`, `/team` and `/trigger` command trees
//! (ScoreboardCommand, TeamCommand, TriggerCommand).
use super::{Dispatcher, args::ArgType, vanilla::GAMEMASTERS};

const HOLDERS: ArgType = ArgType::ScoreHolder { multiple: true };
const HOLDER: ArgType = ArgType::ScoreHolder { multiple: false };

/// `addNumberFormats`: blank | fixed <contents> | styled <style>, and the
/// node itself clears the format.
fn number_formats(d: &mut Dispatcher, top: usize, id: &'static str) {
    d.executes(top, id);
    let blank = d.literal(top, "blank");
    d.executes(blank, id);
    let fixed = d.literal(top, "fixed");
    let contents = d.argument(fixed, "contents", ArgType::Component);
    d.executes(contents, id);
    let styled = d.literal(top, "styled");
    let style = d.argument(styled, "style", ArgType::Style);
    d.executes(style, id);
}

pub fn register(d: &mut Dispatcher, root: usize) {
    scoreboard(d, root);
    team(d, root);

    // trigger <objective> [add|set <value>]
    let trigger = d.literal(root, "trigger");
    let objective = d.argument(trigger, "objective", ArgType::Objective);
    d.suggests(objective, super::Suggests::Triggers);
    d.executes(objective, "trigger");
    for mode in ["add", "set"] {
        let m = d.literal(objective, mode);
        let v = d.argument(
            m,
            "value",
            ArgType::Integer {
                min: i32::MIN,
                max: i32::MAX,
            },
        );
        d.executes(
            v,
            if mode == "add" {
                "trigger.add"
            } else {
                "trigger.set"
            },
        );
    }
}

fn scoreboard(d: &mut Dispatcher, root: usize) {
    let sb = d.literal(root, "scoreboard");
    d.requires(sb, GAMEMASTERS);

    let objectives = d.literal(sb, "objectives");
    let list = d.literal(objectives, "list");
    d.executes(list, "scoreboard.objectives.list");
    let add = d.literal(objectives, "add");
    let name = d.argument(add, "objective", ArgType::Word);
    let criteria = d.argument(name, "criteria", ArgType::Criteria);
    d.executes(criteria, "scoreboard.objectives.add");
    let display = d.argument(criteria, "displayName", ArgType::Component);
    d.executes(display, "scoreboard.objectives.add");
    let modify = d.literal(objectives, "modify");
    let objective = d.argument(modify, "objective", ArgType::Objective);
    let dn = d.literal(objective, "displayname");
    let display = d.argument(dn, "displayName", ArgType::Component);
    d.executes(display, "scoreboard.objectives.modify.displayname");
    let rt = d.literal(objective, "rendertype");
    for kind in ["integer", "hearts"] {
        let k = d.literal(rt, kind);
        d.executes(k, "scoreboard.objectives.modify.rendertype");
    }
    let au = d.literal(objective, "displayautoupdate");
    let v = d.argument(au, "value", ArgType::Bool);
    d.executes(v, "scoreboard.objectives.modify.displayautoupdate");
    let nf = d.literal(objective, "numberformat");
    number_formats(d, nf, "scoreboard.objectives.modify.numberformat");
    let remove = d.literal(objectives, "remove");
    let objective = d.argument(remove, "objective", ArgType::Objective);
    d.executes(objective, "scoreboard.objectives.remove");
    let setdisplay = d.literal(objectives, "setdisplay");
    let slot = d.argument(setdisplay, "slot", ArgType::ScoreboardSlot);
    d.executes(slot, "scoreboard.objectives.setdisplay");
    let objective = d.argument(slot, "objective", ArgType::Objective);
    d.executes(objective, "scoreboard.objectives.setdisplay");

    let players = d.literal(sb, "players");
    let list = d.literal(players, "list");
    d.executes(list, "scoreboard.players.list");
    let target = d.argument(list, "target", HOLDER);
    d.executes(target, "scoreboard.players.list");
    // set / add / remove <targets> <objective> <score>
    for (word, min, id) in [
        ("set", i32::MIN, "scoreboard.players.set"),
        ("add", 0, "scoreboard.players.add"),
        ("remove", 0, "scoreboard.players.remove"),
    ] {
        let w = d.literal(players, word);
        let t = d.argument(w, "targets", HOLDERS);
        let o = d.argument(t, "objective", ArgType::Objective);
        let s = d.argument(o, "score", ArgType::Integer { min, max: i32::MAX });
        d.executes(s, id);
    }
    let get = d.literal(players, "get");
    let t = d.argument(get, "target", HOLDER);
    let o = d.argument(t, "objective", ArgType::Objective);
    d.executes(o, "scoreboard.players.get");
    let reset = d.literal(players, "reset");
    let t = d.argument(reset, "targets", HOLDERS);
    d.executes(t, "scoreboard.players.reset");
    let o = d.argument(t, "objective", ArgType::Objective);
    d.executes(o, "scoreboard.players.reset");
    let enable = d.literal(players, "enable");
    let t = d.argument(enable, "targets", HOLDERS);
    let o = d.argument(t, "objective", ArgType::Objective);
    d.suggests(o, super::Suggests::EnableTriggers);
    d.executes(o, "scoreboard.players.enable");
    let display = d.literal(players, "display");
    let name = d.literal(display, "name");
    let t = d.argument(name, "targets", HOLDERS);
    let o = d.argument(t, "objective", ArgType::Objective);
    d.executes(o, "scoreboard.players.display.name");
    let n = d.argument(o, "name", ArgType::Component);
    d.executes(n, "scoreboard.players.display.name");
    let nf = d.literal(display, "numberformat");
    let t = d.argument(nf, "targets", HOLDERS);
    let o = d.argument(t, "objective", ArgType::Objective);
    number_formats(d, o, "scoreboard.players.display.numberformat");
    let operation = d.literal(players, "operation");
    let t = d.argument(operation, "targets", HOLDERS);
    let to = d.argument(t, "targetObjective", ArgType::Objective);
    let op = d.argument(to, "operation", ArgType::Operation);
    let source = d.argument(op, "source", HOLDERS);
    let so = d.argument(source, "sourceObjective", ArgType::Objective);
    d.executes(so, "scoreboard.players.operation");
}

fn team(d: &mut Dispatcher, root: usize) {
    let team = d.literal(root, "team");
    d.requires(team, GAMEMASTERS);
    let list = d.literal(team, "list");
    d.executes(list, "team.list");
    let t = d.argument(list, "team", ArgType::Team);
    d.executes(t, "team.list");
    let add = d.literal(team, "add");
    let t = d.argument(add, "team", ArgType::Word);
    d.executes(t, "team.add");
    let dn = d.argument(t, "displayName", ArgType::Component);
    d.executes(dn, "team.add");
    let remove = d.literal(team, "remove");
    let t = d.argument(remove, "team", ArgType::Team);
    d.executes(t, "team.remove");
    let empty = d.literal(team, "empty");
    let t = d.argument(empty, "team", ArgType::Team);
    d.executes(t, "team.empty");
    let join = d.literal(team, "join");
    let t = d.argument(join, "team", ArgType::Team);
    d.executes(t, "team.join");
    let m = d.argument(t, "members", HOLDERS);
    d.executes(m, "team.join");
    let leave = d.literal(team, "leave");
    let m = d.argument(leave, "members", HOLDERS);
    d.executes(m, "team.leave");

    let modify = d.literal(team, "modify");
    let t = d.argument(modify, "team", ArgType::Team);
    let dn = d.literal(t, "displayName");
    let v = d.argument(dn, "displayName", ArgType::Component);
    d.executes(v, "team.modify.displayname");
    let color = d.literal(t, "color");
    let reset = d.literal(color, "reset");
    d.executes(reset, "team.modify.color.reset");
    let v = d.argument(color, "value", ArgType::TeamColor);
    d.executes(v, "team.modify.color");
    for (word, id) in [
        ("friendlyFire", "team.modify.friendlyfire"),
        ("seeFriendlyInvisibles", "team.modify.seefriendlyinvisibles"),
    ] {
        let w = d.literal(t, word);
        let v = d.argument(w, "allowed", ArgType::Bool);
        d.executes(v, id);
    }
    for (word, id, values) in [
        (
            "nametagVisibility",
            "team.modify.nametagvisibility",
            ["never", "hideForOtherTeams", "hideForOwnTeam", "always"],
        ),
        (
            "deathMessageVisibility",
            "team.modify.deathmessagevisibility",
            ["never", "hideForOtherTeams", "hideForOwnTeam", "always"],
        ),
        (
            "collisionRule",
            "team.modify.collisionrule",
            ["never", "pushOwnTeam", "pushOtherTeams", "always"],
        ),
    ] {
        let w = d.literal(t, word);
        for value in values {
            let v = d.literal(w, value);
            d.executes(v, id);
        }
    }
    for (word, id) in [
        ("prefix", "team.modify.prefix"),
        ("suffix", "team.modify.suffix"),
    ] {
        let w = d.literal(t, word);
        let v = d.argument(w, word, ArgType::Component);
        d.executes(v, id);
    }
}
