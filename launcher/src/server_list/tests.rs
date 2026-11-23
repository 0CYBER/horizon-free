use super::*;
use crate::backend::Links;
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "horizon-launcher-servers-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn s(v: &str) -> Tag {
    Tag::String(v.into())
}

/// A servers.dat as the game writes it: one shown server with a picture and the fields the
/// launcher doesn't use, then one joined through Direct Connection.
fn vanilla_file(home: &Path) -> Vec<u8> {
    let icon = base64::engine::general_purpose::STANDARD.encode(crate::test_support::png(64, 64));
    let root = NamedTag {
        name: "".into(),
        tag: Tag::compound([(
            "servers",
            Tag::List(vec![
                Tag::compound([
                    ("name", s("Friends")),
                    ("ip", s("friends.example:25570")),
                    ("icon", s(&icon)),
                    ("acceptTextures", Tag::Byte(1)),
                    ("acceptedCodeOfConduct", Tag::Int(-12345)),
                    ("hidden", Tag::Byte(0)),
                ]),
                Tag::compound([
                    ("name", s("Minecraft Server")),
                    ("ip", s("direct.example")),
                    ("hidden", Tag::Byte(1)),
                ]),
            ]),
        )]),
    };
    let bytes = nbt::write_named(&root, Limits::default()).unwrap();
    fs::write(home.join(FILE), &bytes).unwrap();
    bytes
}

fn entries(home: &Path) -> Vec<Tag> {
    let bytes = fs::read(home.join(FILE)).unwrap();
    match nbt::read_named(&bytes, Limits::default())
        .unwrap()
        .0
        .tag
        .get("servers")
    {
        Some(Tag::List(list)) => list.clone(),
        other => panic!("no servers list: {other:?}"),
    }
}

#[test]
fn reads_the_shown_servers_with_their_pictures() {
    let home = scratch("reads");
    vanilla_file(home.as_path());
    let list = List::load(home.as_path());
    let servers = list.servers();
    let shown: Vec<(&str, &str)> = servers
        .iter()
        .map(|s| (s.name.as_str(), s.ip.as_str()))
        .collect();
    assert_eq!(shown, [("Friends", "friends.example:25570")]);
    let icon = servers[0].icon.as_deref().unwrap();
    assert_eq!(crate::test_support::dimensions(icon), (64, 64));
    // No file, or one that isn't NBT: an empty list, as in the game.
    assert!(List::load(&home.as_path().join("none"))
        .servers()
        .is_empty());
    fs::write(home.as_path().join(FILE), b"not nbt").unwrap();
    assert!(List::load(home.as_path()).servers().is_empty());
}

/// A servers.dat a modpack ships can hold any picture: one wider than any GPU's texture limit
/// (20000 × 1, under a kilobyte) is left out, never handed to GPUI, and a big one comes small.
#[test]
fn a_server_picture_is_listed_small_or_not_at_all() {
    let home = scratch("pictures");
    vanilla_file(home.as_path());
    let mut list = List::load(home.as_path());
    let base64 = |png: Vec<u8>| base64::engine::general_purpose::STANDARD.encode(png);
    let wide = base64(crate::test_support::png(20000, 1));
    list.entries[0].insert(key("icon"), s(&wide));
    assert_eq!(list.servers()[0].icon, None);
    let big = base64(crate::test_support::png(512, 512));
    list.entries[0].insert(key("icon"), s(&big));
    let icon = list.servers()[0].icon.clone().unwrap();
    assert_eq!(crate::test_support::dimensions(&icon), (128, 128));
    fs::remove_dir_all(&home).unwrap();
}

#[test]
fn adding_and_removing_keeps_every_other_field_and_the_hidden_servers() {
    let home = scratch("keeps");
    let original = vanilla_file(home.as_path());
    let before = entries(home.as_path());

    let mut list = List::load(home.as_path());
    list.add("  Hub ", " hub.example ").unwrap();
    assert!(list.add("x", "  ").is_err());
    list.save(home.as_path()).unwrap();
    // The game's own backup of what was there.
    assert_eq!(
        fs::read(home.as_path().join("servers.dat_old")).unwrap(),
        original
    );
    assert!(!home.as_path().join("servers.dat_new").exists());

    let after = entries(home.as_path());
    assert_eq!(after.len(), 3);
    assert_eq!(
        after[0], before[0],
        "the first server is written back untouched"
    );
    assert_eq!(
        after[1],
        Tag::compound([
            ("name", s("Hub")),
            ("ip", s("hub.example")),
            ("hidden", Tag::Byte(0)),
        ]),
        "a new server goes after the shown ones, before the hidden ones"
    );
    assert_eq!(after[2], before[1], "the hidden server stays");

    let mut list = List::load(home.as_path());
    let (i, found) = find(&list, "friends").unwrap();
    assert_eq!((i, found.ip.as_str()), (0, "friends.example:25570"));
    assert_eq!(find(&list, "2").unwrap().1.name, "Hub");
    assert_eq!(find(&list, "HUB.example").unwrap().0, 1);
    assert!(find(&list, "3").is_err(), "the hidden one can't be named");
    assert_eq!(list.remove(0).unwrap().name, "Friends");
    assert!(list.remove(5).is_err());
    list.save(home.as_path()).unwrap();
    assert_eq!(
        entries(home.as_path()),
        vec![after[1].clone(), before[1].clone()]
    );
}

#[test]
fn a_confirmed_removal_never_removes_another_server() {
    let mut list = List {
        entries: vec![entry("A", "a.example"), entry("C", "c.example")],
    };
    // The row showed B second; B was removed since, and C is second now.
    assert!(!list.remove_confirmed(1, "B", "b.example"));
    let names = |l: &List| l.servers().into_iter().map(|s| s.name).collect::<Vec<_>>();
    assert_eq!(names(&list), ["A", "C"]);
    // C, confirmed third before a server above it went, is still removed.
    assert!(list.remove_confirmed(2, "C", "c.example"));
    assert_eq!(names(&list), ["A"]);
}

#[test]
fn a_server_without_a_name_gets_the_games_default() {
    let home = scratch("default");
    let mut list = List::load(home.as_path());
    list.add("", "play.example").unwrap();
    list.save(home.as_path()).unwrap();
    assert!(!home.as_path().join("servers.dat_old").exists());
    assert_eq!(
        List::load(home.as_path()).servers()[0].name,
        "Minecraft Server"
    );
}

/// The official server as the launcher knows it before links.json is read.
fn add_official_default(home: &Path) -> Result<(), String> {
    let links = Links::default();
    add_official(
        home,
        &links.official_server,
        &links.official_server_previous,
    )
}

fn ips(home: &Path) -> Vec<String> {
    List::load(home)
        .servers()
        .into_iter()
        .map(|s| s.ip)
        .collect()
}

fn shown(home: &Path) -> Vec<(String, String)> {
    List::load(home)
        .servers()
        .into_iter()
        .map(|s| (s.name, s.ip))
        .collect()
}

#[test]
fn the_official_server_is_put_on_top_once() {
    let (name, address) = (OFFICIAL_NAME, OFFICIAL_ADDRESS);
    // A fresh list: just the official server.
    let home = scratch("official-fresh");
    add_official_default(&home).unwrap();
    assert_eq!(shown(&home), [(name.into(), address.into())]);
    assert_eq!(
        fs::read_to_string(home.join(OFFICIAL_MARKER)).unwrap(),
        format!("{address}\n")
    );
    // An existing list: on top of the shown ones, every entry and field kept.
    let home = scratch("official-existing");
    let before = entries_of(&vanilla_file(&home));
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home), [address, "friends.example:25570"]);
    assert_eq!(
        entries(&home)[1..],
        before[..],
        "the player's entries stay as they were"
    );
    // Once only.
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home), [address, "friends.example:25570"]);
    // Removed by the player: never back.
    let mut list = List::load(&home);
    list.remove(0).unwrap();
    list.save(&home).unwrap();
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home), ["friends.example:25570"]);
    // Already there under the player's own name and spelling: not duplicated.
    let home = scratch("official-present");
    let mut list = List::load(&home);
    list.add("SMP", "Play.PlayHorizon.dev:25565").unwrap();
    list.save(&home).unwrap();
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home), ["Play.PlayHorizon.dev:25565"]);
    // A list that can't be read is left alone, and offered again once it can be.
    let home = scratch("official-broken");
    fs::write(home.join(FILE), b"not nbt").unwrap();
    assert!(add_official_default(&home).is_err());
    assert_eq!(fs::read(home.join(FILE)).unwrap(), b"not nbt");
    assert!(!home.join(OFFICIAL_MARKER).exists());
    // No official server in links.json: nothing offered, nothing written.
    let home = scratch("official-none");
    add_official(&home, " ", &["horizon.bunchy7s.dev".into()]).unwrap();
    assert!(!home.join(FILE).exists());
    assert!(!home.join(OFFICIAL_MARKER).exists());
}

/// A list offered the official server at its old address, as launchers and games before the move
/// left it: the marker names the old address.
fn offered_at(home: &Path, servers: &[(&str, &str)], marker: &str) {
    let mut list = List::load(home);
    for (name, ip) in servers {
        list.add(name, ip).unwrap();
    }
    list.save(home).unwrap();
    fs::write(home.join(OFFICIAL_MARKER), format!("{marker}\n")).unwrap();
}

#[test]
fn a_moved_official_server_moves_the_players_entry_once() {
    let new = OFFICIAL_ADDRESS;
    // Under the player's own name, in any spelling of the old address: the address moves, the
    // name, the picture and the place stay.
    let home = scratch("moved");
    offered_at(
        &home,
        &[
            ("Friends", "friends.example"),
            ("Our SMP", "Horizon.Bunchy7s.DEV:25565"),
        ],
        "horizon.bunchy7s.dev",
    );
    let mut list = List::load(&home);
    list.entries[1].insert(key("icon"), s("AAAA"));
    list.save(&home).unwrap();
    add_official_default(&home).unwrap();
    assert_eq!(
        shown(&home),
        [
            ("Friends".into(), "friends.example".into()),
            ("Our SMP".into(), new.into())
        ]
    );
    assert_eq!(text(&List::load(&home).entries[1], "icon"), "AAAA");
    assert_eq!(
        fs::read_to_string(home.join(OFFICIAL_MARKER)).unwrap(),
        format!("{new}\n")
    );
    // Once: nothing changes on the next look, not even the file.
    let file = fs::read(home.join(FILE)).unwrap();
    add_official_default(&home).unwrap();
    assert_eq!(fs::read(home.join(FILE)).unwrap(), file);
    // An old address the player adds back after the move is theirs.
    let mut list = List::load(&home);
    list.add("Old", "horizon.bunchy7s.dev").unwrap();
    list.save(&home).unwrap();
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home)[2], "horizon.bunchy7s.dev");

    // The new address already in the list: the old entry goes instead of making a second one.
    let home = scratch("moved-present");
    offered_at(
        &home,
        &[
            ("Old", "horizon.bunchy7s.dev"),
            ("New", "play.playhorizon.dev"),
        ],
        "horizon.bunchy7s.dev",
    );
    add_official_default(&home).unwrap();
    assert_eq!(shown(&home), [("New".into(), new.into())]);
    // Two entries for the old address: one is kept, moved.
    let home = scratch("moved-twice");
    offered_at(
        &home,
        &[
            ("A", "horizon.bunchy7s.dev"),
            ("B", "horizon.bunchy7s.dev:25565"),
        ],
        "horizon.bunchy7s.dev",
    );
    add_official_default(&home).unwrap();
    assert_eq!(shown(&home), [("A".into(), new.into())]);

    // Removed by the player before the move: not offered again.
    let home = scratch("moved-removed");
    offered_at(
        &home,
        &[("Friends", "friends.example")],
        "horizon.bunchy7s.dev",
    );
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home), ["friends.example"]);
    // Another port is another server: left alone.
    let home = scratch("moved-port");
    offered_at(
        &home,
        &[("Test", "horizon.bunchy7s.dev:25570")],
        "horizon.bunchy7s.dev",
    );
    add_official_default(&home).unwrap();
    assert_eq!(ips(&home), ["horizon.bunchy7s.dev:25570"]);
    // A list that was never offered it but has the old address: moved, nothing added.
    let home = scratch("moved-unoffered");
    let mut list = List::load(&home);
    list.add("SMP", "horizon.bunchy7s.dev").unwrap();
    list.save(&home).unwrap();
    add_official_default(&home).unwrap();
    assert_eq!(shown(&home), [("SMP".into(), new.into())]);
}

#[test]
fn the_official_address_comes_from_links_json() {
    // The defaults: the new host, and the old one to move from.
    let defaults = Links::default();
    assert_eq!(defaults.official_server, "play.playhorizon.dev");
    assert_eq!(defaults.official_server_previous, ["horizon.bunchy7s.dev"]);
    // The file on main says the same.
    let file = Links::parse(include_str!("../../links.json"));
    assert_eq!(file, defaults);
    // A later move is a links.json edit: the list follows it, from either earlier address.
    let links = Links::parse(
        r#"{"official_server": " next.example ",
            "official_server_previous": ["play.playhorizon.dev", "horizon.bunchy7s.dev", ""]}"#,
    );
    assert_eq!(links.official_server, "next.example");
    assert_eq!(
        links.official_server_previous,
        ["play.playhorizon.dev", "horizon.bunchy7s.dev"]
    );
    let home = scratch("links-move");
    offered_at(
        &home,
        &[
            ("SMP", "play.playhorizon.dev"),
            ("Old", "horizon.bunchy7s.dev"),
        ],
        "play.playhorizon.dev",
    );
    add_official(
        &home,
        &links.official_server,
        &links.official_server_previous,
    )
    .unwrap();
    assert_eq!(shown(&home), [("SMP".into(), "next.example".into())]);
    // A fresh list gets the address links.json gives.
    let home = scratch("links-fresh");
    add_official(
        &home,
        &links.official_server,
        &links.official_server_previous,
    )
    .unwrap();
    assert_eq!(
        shown(&home),
        [(OFFICIAL_NAME.into(), "next.example".into())]
    );
    // A previous list that is not a list of addresses keeps the built-in one.
    for bad in [
        r#"{"official_server_previous": "x"}"#,
        r#"{"official_server_previous": [1]}"#,
    ] {
        assert_eq!(
            Links::parse(bad).official_server_previous,
            defaults.official_server_previous
        );
    }
    // Empty: no official server.
    assert_eq!(
        Links::parse(r#"{"official_server": ""}"#).official_server,
        ""
    );
}

fn entries_of(bytes: &[u8]) -> Vec<Tag> {
    match nbt::read_named(bytes, Limits::default())
        .unwrap()
        .0
        .tag
        .get("servers")
    {
        Some(Tag::List(list)) => list.clone(),
        other => panic!("no servers list: {other:?}"),
    }
}
