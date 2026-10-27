use super::*;
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
    let png = [0x89, b'P', b'N', b'G', 1, 2, 3];
    let icon = base64::engine::general_purpose::STANDARD.encode(png);
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
    assert_eq!(
        list.servers(),
        vec![Server {
            name: "Friends".into(),
            ip: "friends.example:25570".into(),
            icon: Some(vec![0x89, b'P', b'N', b'G', 1, 2, 3]),
        }]
    );
    // No file, or one that isn't NBT: an empty list, as in the game.
    assert!(List::load(&home.as_path().join("none"))
        .servers()
        .is_empty());
    fs::write(home.as_path().join(FILE), b"not nbt").unwrap();
    assert!(List::load(home.as_path()).servers().is_empty());
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
