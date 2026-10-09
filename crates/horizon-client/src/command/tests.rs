use super::{args::*, vanilla, *};

fn reg() -> Registries {
    let mut r = Registries {
        permission: 4,
        ..Registries::default()
    };
    r.items = vec![
        "minecraft:diamond".into(),
        "minecraft:diamond_sword".into(),
        "minecraft:stone".into(),
    ];
    r.blocks.insert("minecraft:stone".into(), vec![]);
    r.blocks.insert(
        "minecraft:oak_stairs".into(),
        vec![
            ("facing".into(), vec!["north".into(), "south".into()]),
            ("half".into(), vec!["top".into(), "bottom".into()]),
        ],
    );
    r.summonable = vec!["minecraft:pig".into(), "minecraft:zombie".into()];
    r.entity_types = vec![
        "minecraft:pig".into(),
        "minecraft:player".into(),
        "minecraft:zombie".into(),
    ];
    r.players = vec!["Player".into()];
    r.dimensions = vec!["minecraft:overworld".into()];
    r.time_markers = vec!["minecraft:day".into(), "minecraft:noon".into()];
    r
}
fn run(input: &str) -> CmdResult<(String, Vec<String>)> {
    let reg = reg();
    let parse = vanilla::dispatcher().parse(input, &reg);
    let (ctx, id) = parse.executable()?;
    Ok((
        id.to_owned(),
        ctx.args.iter().map(|a| a.name.clone()).collect(),
    ))
}
fn suggestions(input: &str) -> Vec<String> {
    let reg = reg();
    let d = vanilla::dispatcher();
    let parse = d.parse(input, &reg);
    d.suggestions(&parse, input.len(), &reg)
        .1
        .into_iter()
        .map(|s| s.text)
        .collect()
}

#[test]
fn reader_matches_brigadier() {
    let mut r = Reader::new("12 -3.5 abc \"q\\\"x\" true");
    assert_eq!(r.read_int().unwrap(), 12);
    r.skip();
    assert_eq!(r.read_double().unwrap(), -3.5);
    r.skip();
    assert_eq!(r.read_unquoted_string(), "abc");
    r.skip();
    assert_eq!(r.read_string().unwrap(), "q\"x");
    r.skip();
    assert!(r.read_boolean().unwrap());
    let e = Reader::new("1.5").read_int().unwrap_err();
    assert_eq!(e.message.plain(), "parsing.int.invalid");
    assert_eq!(e.context().unwrap(), "<--[HERE]");
}
#[test]
fn parses_vanilla_commands() {
    assert_eq!(run("gamemode creative").unwrap().0, "gamemode");
    assert_eq!(
        run("gamemode creative @s").unwrap().1,
        ["gamemode", "target"]
    );
    assert_eq!(
        run("give @s minecraft:diamond 64").unwrap().1,
        ["targets", "item", "count"]
    );
    assert_eq!(run("tp 1 2 3").unwrap().0, "teleport.location");
    assert_eq!(run("tp Player").unwrap().0, "teleport.destination");
    assert_eq!(run("tp @s ~ ~10 ~").unwrap().0, "teleport.targets.location");
    assert_eq!(
        run("teleport @e[type=pig,limit=2] ^ ^ ^5 90 0").unwrap().0,
        "teleport.targets.location"
    );
    assert_eq!(run("time set 1000").unwrap().0, "time.set");
    assert_eq!(run("time set 1d").unwrap().0, "time.set");
    assert_eq!(run("time set day").unwrap().0, "time.set.marker");
    assert_eq!(run("time query gametime").unwrap().0, "time.query.gametime");
    assert_eq!(run("weather rain 10s").unwrap().0, "weather");
    assert_eq!(run("xp add @s 5 levels").unwrap().0, "experience");
    assert_eq!(run("gamerule keep_inventory true").unwrap().0, "gamerule");
    assert_eq!(
        run("gamerule minecraft:random_tick_speed 10").unwrap().0,
        "gamerule"
    );
    assert_eq!(run("fill ~ ~ ~ ~1 ~1 ~1 stone hollow").unwrap().0, "fill");
    assert_eq!(
        run("fill 0 0 0 1 1 1 minecraft:oak_stairs[facing=north] replace stone")
            .unwrap()
            .0,
        "fill"
    );
    assert_eq!(
        run("clone 0 0 0 1 1 1 5 5 5 masked move").unwrap().0,
        "clone"
    );
    assert_eq!(
        run("setblock ~ ~1 ~ oak_stairs[half=top] keep").unwrap().0,
        "setblock"
    );
    assert_eq!(run("summon zombie ~ ~ ~ {NoAI:1b}").unwrap().0, "summon");
    assert_eq!(run("difficulty hard").unwrap().0, "difficulty");
    assert_eq!(run("say hello @p").unwrap().0, "say");
    let reg = reg();
    let parse =
        vanilla::dispatcher().parse("clone 0 0 0 1 1 1 5 5 5 strict filtered stone force", &reg);
    let (ctx, _) = parse.executable().unwrap();
    assert!(ctx.has("strict") && ctx.has("filtered") && ctx.has("force") && !ctx.has("move"));
}
#[test]
fn vanilla_error_messages() {
    let e = run("foo").unwrap_err();
    assert_eq!(e.message.plain(), "command.unknown.command");
    assert_eq!(e.cursor, 0);
    let e = run("gamemode").unwrap_err();
    assert_eq!(e.message.plain(), "command.unknown.command");
    let e = run("gamemode creatve").unwrap_err();
    assert_eq!(e.message.plain(), "argument.gamemode.invalid");
    assert_eq!(e.cursor, 9);
    let e = run("give @s minecraft:dirt").unwrap_err();
    assert_eq!(e.message.plain(), "argument.item.id.invalid");
    let e = run("give @s diamond 0").unwrap_err();
    assert_eq!(e.message.plain(), "argument.integer.low");
    let e = run("gamemode creative @e").unwrap_err();
    assert_eq!(e.message.plain(), "argument.player.entities");
    let e = run("setblock 1 2").unwrap_err();
    assert_eq!(e.message.plain(), "argument.pos3d.incomplete");
    let e = run("setblock 1 2 3 oak_stairs[facing=up]").unwrap_err();
    assert_eq!(e.message.plain(), "argument.block.property.invalid");
}
#[test]
fn suggestions_like_vanilla() {
    let root = suggestions("");
    assert!(root.contains(&"gamemode".to_owned()) && root.contains(&"tp".to_owned()));
    assert_eq!(suggestions("game"), ["gamemode", "gamerule"]);
    assert_eq!(
        suggestions("gamemode "),
        ["adventure", "creative", "spectator", "survival"]
    );
    assert_eq!(suggestions("gamemode c"), ["creative"]);
    assert_eq!(
        suggestions("give @s dia"),
        ["minecraft:diamond", "minecraft:diamond_sword"]
    );
    // Underscore-separated parts match too (SharedSuggestionProvider.matchesSubStr).
    assert_eq!(suggestions("give @s sword"), ["minecraft:diamond_sword"]);
    assert_eq!(suggestions("kill @"), ["@a", "@e", "@n", "@p", "@r", "@s"]);
    assert_eq!(suggestions("kill @e"), ["["]);
    assert_eq!(suggestions("kill @e[ty"), ["type="]);
    assert!(suggestions("kill @e[type=").contains(&"minecraft:zombie".to_owned()));
    assert_eq!(suggestions("kill @e[type=pig"), [",", "]"]);
    assert_eq!(suggestions("setblock 0 0 0 oak_stairs[fa"), ["facing="]);
    assert_eq!(suggestions("setblock 0 0 0 oak_stairs[facing=n"), ["north"]);
    assert_eq!(
        suggestions("tp "),
        [
            "@a", "@e", "@n", "@p", "@r", "@s", "Player", "~", "~ ~", "~ ~ ~"
        ]
    );
    assert_eq!(suggestions("time set 5"), ["d", "s", "t"]);
    assert!(suggestions("time set ").contains(&"minecraft:day".to_owned()));
}
#[test]
fn smart_usage_matches_brigadier() {
    let d = vanilla::dispatcher();
    let usage = |name: &str| {
        d.smart_usage(0, 4)
            .into_iter()
            .find(|(n, _)| d.nodes[*n].name() == name)
            .unwrap()
            .1
    };
    assert_eq!(usage("gamemode"), "gamemode <gamemode> [<target>]");
    assert_eq!(usage("give"), "give <targets> <item> [<count>]");
    assert_eq!(usage("kill"), "kill [<targets>]");
    assert_eq!(usage("weather"), "weather (clear|rain|thunder)");
    assert_eq!(usage("tp"), "tp -> teleport");
    assert_eq!(
        usage("difficulty"),
        "difficulty [peaceful|easy|normal|hard]"
    );
}
#[test]
fn local_coordinates_follow_view() {
    // Facing south (yaw 0) moves +Z.
    let p = Coordinates::Local([0.0, 0.0, 2.0]).position([0.0, 64.0, 0.0], [0.0, 0.0]);
    assert!((p[2] - 2.0).abs() < 1e-6 && p[0].abs() < 1e-6);
    // ^1 (left) while facing south is +X.
    let p = Coordinates::Local([1.0, 0.0, 0.0]).position([0.0, 64.0, 0.0], [0.0, 0.0]);
    assert!((p[0] - 1.0).abs() < 1e-6, "{p:?}");
    let w = Coordinates::World([
        Coord {
            relative: true,
            value: 1.0,
        },
        Coord {
            relative: false,
            value: 5.0,
        },
        Coord {
            relative: true,
            value: 0.0,
        },
    ]);
    assert_eq!(w.position([10.0, 64.0, 3.0], [0.0, 0.0]), [11.0, 5.0, 3.0]);
}

#[test]
fn data_and_tag_commands_parse() {
    let (id, args) = run("data get entity @s Pos 2.5").unwrap();
    assert_eq!(id, "data.get");
    assert_eq!(args, ["target", "path", "scale"]);
    assert_eq!(run("data get block ~ ~ ~ Items[0]").unwrap().0, "data.get");
    assert_eq!(run("data get storage my:store").unwrap().0, "data.get");
    assert_eq!(run("data merge storage a:b {x:1}").unwrap().0, "data.merge");
    assert_eq!(
        run("data remove entity @s foo.bar[1]").unwrap().0,
        "data.remove"
    );
    let (id, args) = run("data modify storage a:b x.y insert 2 from entity @s Pos[0]").unwrap();
    assert_eq!(id, "data.modify");
    assert_eq!(
        args,
        ["target", "targetPath", "index", "source", "sourcePath"]
    );
    assert_eq!(
        run("data modify block 1 2 3 Text1 set string storage a:b x 1 3")
            .unwrap()
            .0,
        "data.modify"
    );
    assert_eq!(
        run("data modify storage a:b l append value [1,2]")
            .unwrap()
            .0,
        "data.modify"
    );
    assert!(run("data modify storage a:b l explode value 1").is_err());
    assert!(run("data get storage a:b {broken").is_err());
    assert!(run("data merge storage a:b 5").is_err());
    assert_eq!(run("tag @e[tag=!x] add foo").unwrap().0, "tag.add");
    assert_eq!(run("tag @s list").unwrap().0, "tag.list");
    // Selector options added for scoreboards and NBT.
    assert!(run("kill @e[scores={a=1..},team=red,nbt={x:1b},tag=t]").is_ok());
    assert!(run("kill @e[scores={a=1..]").is_err());
}
