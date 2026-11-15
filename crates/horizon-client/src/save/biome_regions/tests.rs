use super::*;
use std::fs;

fn point(biome: &str, low: f32) -> mod_api::BiomePoint {
    mod_api::BiomePoint {
        biome: biome.into(),
        temperature: (low, 0.1),
        humidity: (-0.3, 0.7),
        continentalness: (-1.9, 1.3),
        erosion: (0.15, 0.25),
        depth: (0.0, 1.0),
        weirdness: (-2.0, 2.0),
        offset: 0.375,
    }
}

pub(super) fn declaration(
    mod_id: &str,
    id: &str,
    dimension: &str,
    weight: u32,
) -> BiomeRegionDeclaration {
    BiomeRegionDeclaration {
        mod_id: mod_id.into(),
        id: id.into(),
        region: mod_api::BiomeRegion {
            dimension: dimension.into(),
            weight,
            points: vec![point(&format!("{mod_id}:hills"), -0.45)],
        },
    }
}

/// Two mods' Overworld regions, sorted as the declarations install them.
fn overworld() -> Vec<BiomeRegionDeclaration> {
    vec![
        declaration("alpha", "alpha:marsh", "minecraft:overworld", 3),
        declaration("zeta", "zeta:canyon", "minecraft:overworld", 1),
    ]
}

pub(super) fn mods<'a>(ids: &[&'a str]) -> BTreeSet<&'a str> {
    ids.iter().copied().collect()
}

/// The Overworld's region list in an encoded record.
fn regions(tag: &mut Tag) -> &mut Vec<Tag> {
    match tag
        .get_mut("dimensions")
        .and_then(|d| d.get_mut("minecraft:overworld"))
        .and_then(|l| l.get_mut("regions"))
    {
        Some(Tag::List(regions)) => regions,
        _ => panic!("the layout has regions"),
    }
}

fn put(tag: &mut Tag, key: &str, value: Tag) {
    if let Tag::Compound(map) = tag {
        map.insert(key.into(), value);
    }
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_new_worlds_record_holds_only_populated_dimensions_in_declared_order() {
    let mut declared = overworld();
    declared.push(declaration("beta", "beta:ash", "minecraft:nether", 7));
    declared.sort_by(|a, b| (&a.mod_id, &a.id).cmp(&(&b.mod_id, &b.id)));
    let record = BiomeRegions::from_declarations(&declared);
    let overworld = record.layout("minecraft:overworld").unwrap();
    assert_eq!(overworld.selector, SelectorParams::current());
    let regions: Vec<_> = overworld
        .regions
        .iter()
        .map(|r| (r.owner.as_str(), r.id.as_str(), r.weight, r.substituted))
        .collect();
    assert_eq!(
        regions,
        [
            ("alpha", "alpha:marsh", 3, false),
            ("zeta", "zeta:canyon", 1, false)
        ]
    );
    assert_eq!(overworld.regions[0].points[0].intervals[0], (-0.45, 0.1));
    let nether = record.layout("minecraft:nether").unwrap();
    assert_eq!(nether.regions.len(), 1);
    assert_eq!(nether.regions[0].id, "beta:ash");
    assert!(record.layout("minecraft:the_end").is_none());
    // No declarations, no layout in any dimension.
    assert_eq!(
        BiomeRegions::from_declarations(&[]),
        BiomeRegions::default()
    );
}

#[test]
fn the_record_reads_back_every_field_bit_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut record = BiomeRegions::from_declarations(&overworld());
    let layout = Arc::make_mut(record.dimensions.get_mut("minecraft:overworld").unwrap());
    layout.selector = SelectorParams {
        version: REGION_SELECTOR_VERSION,
        cell: 64,
        salt: "test:salt".into(),
        vanilla_weight: 12,
    };
    // Values with no short decimal form: a lossy float path would not return these bits.
    layout.regions[0].points[0].intervals[3] = (1.0e-7, f32::from_bits(0x3e4c_cccd));
    layout.regions[0].points[0].offset = f32::from_bits(0x3eaa_aaab);
    layout.regions[1].substituted = true;
    record.write(dir.path()).unwrap();
    assert_eq!(BiomeRegions::read(dir.path()).unwrap(), Some(record));
}

#[test]
fn an_absent_record_is_no_regions() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(BiomeRegions::read(dir.path()), Ok(None));
    assert_eq!(open_with(dir.path(), &mods(&[]), &overworld()), Ok(None));
}

#[test]
fn a_malformed_record_is_refused_and_left_untouched() {
    let good = BiomeRegions::from_declarations(&overworld()).encode();
    let edit = |change: &dyn Fn(&mut Tag)| {
        let mut tag = good.clone();
        change(&mut tag);
        tag
    };
    let variants: Vec<(&str, Option<Tag>)> = vec![
        ("not nbt", None),
        (
            "unknown version",
            Some(edit(&|t: &mut Tag| put(t, "version", Tag::Int(2)))),
        ),
        (
            "ill-typed dimensions",
            Some(edit(&|t: &mut Tag| put(t, "dimensions", Tag::Int(0)))),
        ),
        (
            "missing weight",
            Some(edit(&|t: &mut Tag| {
                if let Tag::Compound(map) = &mut regions(t)[0] {
                    map.remove(&JavaString::from("weight"));
                }
            })),
        ),
        (
            "weight 0",
            Some(edit(&|t: &mut Tag| {
                put(&mut regions(t)[0], "weight", Tag::Int(0))
            })),
        ),
        (
            "point outside -2 to 2",
            Some(edit(&|t: &mut Tag| {
                if let Some(Tag::List(points)) = regions(t)[0].get_mut("points") {
                    put(
                        &mut points[0],
                        "humidity",
                        Tag::List(vec![Tag::Float(0f32.to_bits()), Tag::Float(3f32.to_bits())]),
                    );
                }
            })),
        ),
        ("unsorted", Some(edit(&|t: &mut Tag| regions(t).swap(0, 1)))),
        (
            "duplicate",
            Some(edit(&|t: &mut Tag| {
                let list = regions(t);
                list[1] = list[0].clone();
            })),
        ),
        (
            "substituted for another mod",
            Some(edit(&|t: &mut Tag| {
                put(
                    &mut regions(t)[0],
                    "substituted",
                    Tag::compound([("with", string(VANILLA)), ("missing_mod", string("zeta"))]),
                )
            })),
        ),
    ];
    for (name, tag) in variants {
        let dir = tempfile::tempdir().unwrap();
        let file = path(dir.path());
        match tag {
            Some(tag) => write_dat_durable(&file, tag).unwrap(),
            None => {
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(&file, b"not nbt").unwrap();
            }
        }
        let bytes = fs::read(&file).unwrap();
        let everyone = mods(&["alpha", "zeta"]);
        assert!(BiomeRegions::read(dir.path()).is_err(), "{name}");
        assert!(
            open_with(dir.path(), &everyone, &overworld()).is_err(),
            "{name}"
        );
        assert!(
            choose_with(dir.path(), &mods(&[]), &[], true, true).is_err(),
            "{name}"
        );
        assert!(unchosen_missing(dir.path()).is_empty(), "{name}");
        assert_eq!(fs::read(&file).unwrap(), bytes, "{name}");
        assert_eq!(
            entries(file.parent().unwrap()),
            ["biome_regions.dat"],
            "{name}"
        );
    }
}

#[test]
fn a_missing_mods_region_refuses_the_open_naming_mod_and_region() {
    let dir = tempfile::tempdir().unwrap();
    BiomeRegions::from_declarations(&overworld())
        .write(dir.path())
        .unwrap();
    let only_alpha = vec![overworld().remove(0)];
    let why = open_with(dir.path(), &mods(&["alpha"]), &only_alpha).unwrap_err();
    assert!(
        why.contains("zeta:canyon") && why.contains("mod zeta"),
        "{why}"
    );
    assert!(!why.contains("alpha"), "{why}");
    let record = BiomeRegions::read(dir.path()).unwrap().unwrap();
    assert_eq!(
        record.differences(&mods(&["alpha"]), &only_alpha),
        Differences {
            missing: vec![("zeta".into(), "zeta:canyon".into())],
            ..Differences::default()
        }
    );
}

#[test]
fn choosing_vanilla_is_saved_and_opens_from_then_on() {
    let dir = tempfile::tempdir().unwrap();
    BiomeRegions::from_declarations(&overworld())
        .write(dir.path())
        .unwrap();
    let only_alpha = vec![overworld().remove(0)];
    let alpha = mods(&["alpha"]);
    // Refusing ("missing = refuse") records nothing.
    choose_with(dir.path(), &alpha, &only_alpha, false, false).unwrap();
    assert!(open_with(dir.path(), &alpha, &only_alpha).is_err());
    choose_with(dir.path(), &alpha, &only_alpha, true, false).unwrap();
    let opened = open_with(dir.path(), &alpha, &only_alpha).unwrap().unwrap();
    let layout = opened.layout("minecraft:overworld").unwrap();
    // The region keeps its slot and weight; only the substitution is new.
    assert_eq!(
        layout
            .regions
            .iter()
            .map(|r| (r.id.as_str(), r.weight, r.substituted))
            .collect::<Vec<_>>(),
        [("alpha:marsh", 3, false), ("zeta:canyon", 1, true)]
    );
    // The mod's return is reported, and the substitution stays.
    let everyone = mods(&["alpha", "zeta"]);
    let differences = opened.differences(&everyone, &overworld());
    assert_eq!(differences.returned, ["zeta:canyon"]);
    assert!(differences.missing.is_empty());
    let reopened = open_with(dir.path(), &everyone, &overworld()).unwrap();
    assert_eq!(reopened.unwrap().dimensions, opened.dimensions);
}

#[test]
fn changed_added_and_removed_regions_are_reported_and_the_record_wins() {
    let dir = tempfile::tempdir().unwrap();
    BiomeRegions::from_declarations(&overworld())
        .write(dir.path())
        .unwrap();
    let saved = BiomeRegions::read(dir.path()).unwrap().unwrap();
    let mut now = overworld();
    now[0].region.points[0].offset = 0.5; // alpha's points changed
    now.remove(1); // zeta no longer declares its region
    now.push(declaration("zeta", "zeta:dunes", "minecraft:nether", 2)); // a new region
    let everyone = mods(&["alpha", "zeta"]);
    let mut changed = saved.differences(&everyone, &now).changed;
    changed.sort();
    assert_eq!(changed, ["alpha:marsh", "zeta:canyon", "zeta:dunes"]);
    assert_eq!(
        open_with(dir.path(), &everyone, &now)
            .unwrap()
            .unwrap()
            .dimensions,
        saved.dimensions
    );
    // A changed weight is a difference too.
    let mut heavier = overworld();
    heavier[1].region.weight = 9;
    assert_eq!(
        saved.differences(&everyone, &heavier).changed,
        ["zeta:canyon"]
    );
    assert_eq!(
        saved.differences(&everyone, &overworld()),
        Differences::default()
    );
}

#[test]
fn adopting_takes_installed_regions_and_keeps_saved_weights() {
    let dir = tempfile::tempdir().unwrap();
    let mut saved = BiomeRegions::from_declarations(&overworld());
    let layout = Arc::make_mut(saved.dimensions.get_mut("minecraft:overworld").unwrap());
    layout.regions[0].weight = 40; // the owner's own weight for alpha
    layout.regions[1].substituted = true;
    layout.selector.cell = 64;
    saved.write(dir.path()).unwrap();
    let mut now = overworld();
    now[0].region.points[0].offset = 0.5;
    now.push(declaration("zeta", "zeta:dunes", "minecraft:nether", 2));
    let everyone = mods(&["alpha", "zeta"]);
    choose_with(dir.path(), &everyone, &now, false, true).unwrap();
    let adopted = BiomeRegions::read(dir.path()).unwrap().unwrap();
    let overworld = adopted.layout("minecraft:overworld").unwrap();
    assert_eq!(overworld.selector.cell, 64);
    assert_eq!(
        overworld
            .regions
            .iter()
            .map(|r| (r.id.as_str(), r.weight, r.substituted))
            .collect::<Vec<_>>(),
        [("alpha:marsh", 40, false), ("zeta:canyon", 1, false)]
    );
    assert_eq!(overworld.regions[0].points[0].offset, 0.5);
    let nether = adopted.layout("minecraft:nether").unwrap();
    assert_eq!(
        (nether.regions[0].id.as_str(), nether.regions[0].weight),
        ("zeta:dunes", 2)
    );
    // Adoption preserves the owner's weight 40, so the mod's default weight 3 still differs.
    assert_eq!(
        adopted.differences(&everyone, &now),
        Differences {
            changed: vec!["alpha:marsh".into()],
            ..Differences::default()
        }
    );
}

#[test]
fn the_opt_in_is_offered_only_while_adopting_would_change_the_record() {
    let dir = tempfile::tempdir().unwrap();
    // No record and no regions installed: nothing to adopt.
    assert!(!adoptable_with(dir.path(), &[]));
    // A world made before any region mod: the installed regions are new to it.
    assert!(adoptable_with(dir.path(), &overworld()));
    BiomeRegions::from_declarations(&overworld())
        .write(dir.path())
        .unwrap();
    assert!(!adoptable_with(dir.path(), &overworld()));
    // Changed points, an added region or a removed one are offered; a changed declared
    // weight alone is not, since adopting keeps the saved weight.
    let mut moved = overworld();
    moved[0].region.points[0].offset = 0.5;
    assert!(adoptable_with(dir.path(), &moved));
    let mut added = overworld();
    added.push(declaration("zeta", "zeta:dunes", "minecraft:nether", 2));
    assert!(adoptable_with(dir.path(), &added));
    assert!(adoptable_with(dir.path(), &overworld()[..1]));
    let mut heavier = overworld();
    heavier[1].region.weight = 9;
    assert!(!adoptable_with(dir.path(), &heavier));
    // Adopting makes the record match, and the opt-in is no longer offered.
    choose_with(dir.path(), &mods(&["alpha", "zeta"]), &added, false, true).unwrap();
    assert!(!adoptable_with(dir.path(), &added));
    // A record that does not read is never offered, and stays as it was.
    let file = path(dir.path());
    fs::write(&file, b"not a record").unwrap();
    assert!(!adoptable_with(dir.path(), &added));
    assert_eq!(fs::read(&file).unwrap(), b"not a record");
}

#[test]
fn interrupted_replacement_refuses_to_drop_the_layout() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    fs::create_dir_all(path(dir).parent().unwrap()).unwrap();
    for name in ["biome_regions.dat_old", "biome_regions.dat_new"] {
        let leftover = path(dir).with_file_name(name);
        fs::write(&leftover, b"saved layout").unwrap();
        let why = BiomeRegions::read(dir).unwrap_err();
        assert!(why.contains(name), "{why}");
        assert_eq!(fs::read(&leftover).unwrap(), b"saved layout");
        assert!(!path(dir).exists());
        fs::remove_file(leftover).unwrap();
    }
}

#[test]
fn choices_refuse_a_world_already_open_in_another_session() {
    let scratch = tempfile::tempdir().unwrap();
    let dir = scratch.path();
    let lock = super::super::SessionLock::acquire(dir).unwrap();
    assert!(choose_with(dir, &mods(&[]), &[], true, true).is_err());
    assert!(!path(dir).exists());
    drop(lock);
    assert!(choose_with(dir, &mods(&[]), &[], true, true).is_ok());
}

fn at_mut<'a>(tag: &'a mut Tag, path: &[&str]) -> &'a mut Tag {
    let mut tag = tag;
    for key in path {
        tag = match tag {
            Tag::List(values) => &mut values[key.parse::<usize>().unwrap()],
            _ => tag.get_mut(key).unwrap(),
        };
    }
    tag
}

#[test]
fn every_required_record_field_refuses_missing_or_wrong_types() {
    let good = BiomeRegions::from_declarations(&overworld()).encode();
    let layout = ["dimensions", "minecraft:overworld"];
    let region = ["dimensions", "minecraft:overworld", "regions", "0"];
    let point = [
        "dimensions",
        "minecraft:overworld",
        "regions",
        "0",
        "points",
        "0",
    ];
    let mut paths = vec![vec!["version"], vec!["dimensions"]];
    for field in ["selector", "regions"] {
        paths.push([layout.as_slice(), &[field]].concat());
    }
    for field in ["version", "cell", "salt", "vanilla_weight"] {
        paths.push([layout.as_slice(), &["selector", field]].concat());
    }
    for field in ["owner", "id", "weight", "points"] {
        paths.push([region.as_slice(), &[field]].concat());
    }
    for field in [
        "biome",
        "temperature",
        "humidity",
        "continentalness",
        "erosion",
        "depth",
        "weirdness",
        "offset",
    ] {
        paths.push([point.as_slice(), &[field]].concat());
    }
    for path in paths {
        for missing in [true, false] {
            let mut bad = good.clone();
            let (key, parents) = path.split_last().unwrap();
            let Tag::Compound(fields) = at_mut(&mut bad, parents) else {
                panic!("compound parent")
            };
            if missing {
                fields.remove(&JavaString::from(*key));
            } else {
                fields.insert((*key).into(), Tag::Byte(0));
            }
            let dir = tempfile::tempdir().unwrap();
            let file = super::path(dir.path());
            write_dat_durable(&file, bad).unwrap();
            let before = fs::read(&file).unwrap();
            assert!(
                open_with(dir.path(), &mods(&["alpha", "zeta"]), &overworld()).is_err(),
                "{path:?} missing={missing}"
            );
            assert_eq!(fs::read(&file).unwrap(), before);
            assert_eq!(entries(file.parent().unwrap()), ["biome_regions.dat"]);
        }
    }
}

#[test]
fn saved_selector_identity_climate_and_counts_are_bounded() {
    let good = BiomeRegions::from_declarations(&overworld()).encode();
    let variants = [
        (
            vec!["dimensions", "minecraft:overworld", "selector", "cell"],
            Tag::Int((1 << 30) + 1),
        ),
        (
            vec!["dimensions", "minecraft:overworld", "selector", "cell"],
            Tag::Int(0),
        ),
        (
            vec!["dimensions", "minecraft:overworld", "selector", "version"],
            Tag::Int(2),
        ),
        (
            vec!["dimensions", "minecraft:overworld", "selector", "salt"],
            string(""),
        ),
        (
            vec![
                "dimensions",
                "minecraft:overworld",
                "selector",
                "vanilla_weight",
            ],
            Tag::Int(i32::MAX),
        ),
        (
            vec!["dimensions", "minecraft:overworld", "regions", "0", "owner"],
            string("other"),
        ),
        (
            vec![
                "dimensions",
                "minecraft:overworld",
                "regions",
                "0",
                "points",
                "0",
                "offset",
            ],
            Tag::Float(f32::NAN.to_bits()),
        ),
        (
            vec![
                "dimensions",
                "minecraft:overworld",
                "regions",
                "0",
                "points",
                "0",
                "temperature",
            ],
            Tag::List(vec![Tag::Float(f32::INFINITY.to_bits()), Tag::Float(0)]),
        ),
    ];
    for (path, value) in variants {
        let mut bad = good.clone();
        *at_mut(&mut bad, &path) = value;
        assert!(decode(&bad).is_err(), "{path:?}");
    }
    let mut widest = good.clone();
    *at_mut(
        &mut widest,
        &["dimensions", "minecraft:overworld", "selector", "cell"],
    ) = Tag::Int(1 << 30);
    assert!(decode(&widest).is_ok());
    let mut bad = good.clone();
    let Tag::Compound(dimensions) = bad.get_mut("dimensions").unwrap() else {
        unreachable!()
    };
    dimensions.insert("extra:a".into(), Tag::Int(0));
    dimensions.insert("extra:b".into(), Tag::Int(0));
    assert!(decode(&bad).unwrap_err().contains("exceeds 2"));
    let mut bad = good.clone();
    let values = regions(&mut bad);
    values.resize(MAX_REGIONS + 1, values[0].clone());
    assert!(decode(&bad).unwrap_err().contains("exceeds 1024"));
    let mut bad = good;
    let Tag::List(points) = regions(&mut bad)[0].get_mut("points").unwrap() else {
        unreachable!()
    };
    points.resize(MAX_POINTS + 1, points[0].clone());
    assert!(decode(&bad).unwrap_err().contains("exceeds 16384"));
}

#[test]
fn named_reports_are_remembered_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let mut record = BiomeRegions::from_declarations(&overworld());
    Arc::make_mut(record.dimensions.get_mut("minecraft:overworld").unwrap()).regions[1]
        .substituted = true;
    record.write(dir.path()).unwrap();
    let mut now = overworld();
    now[0].region.points[0].offset = 0.5;
    let opened = open_with(dir.path(), &mods(&["alpha", "zeta"]), &now)
        .unwrap()
        .unwrap();
    assert_eq!(
        opened
            .reported
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["changed:alpha:marsh", "returned:zeta:canyon"]
    );
    let before = fs::read(path(dir.path())).unwrap();
    let backup = fs::read(path(dir.path()).with_extension("dat_old")).unwrap();
    let reopened = open_with(dir.path(), &mods(&["alpha", "zeta"]), &now)
        .unwrap()
        .unwrap();
    assert_eq!(reopened, opened);
    assert_eq!(fs::read(path(dir.path())).unwrap(), before);
    assert_eq!(
        fs::read(path(dir.path()).with_extension("dat_old")).unwrap(),
        backup
    );
    assert!(reopened.layout("minecraft:overworld").unwrap().regions[1].substituted);
}

#[test]
fn oversized_and_overbudget_nbt_refuse_before_materializing_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let file = path(dir.path());
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    let oversized = fs::File::create(&file).unwrap();
    oversized
        .set_len(RECORD_LIMITS.max_bytes as u64 + 1)
        .unwrap();
    assert!(
        BiomeRegions::read(dir.path())
            .unwrap_err()
            .contains("32 MiB")
    );
    // A tiny NBT header claiming too many list nodes must fail before allocating its list.
    let mut raw = b"\x0a\x00\x00\x09\x00\x01x\x01".to_vec();
    raw.extend_from_slice(&(RECORD_LIMITS.max_nodes as i32 + 1).to_be_bytes());
    fs::write(&file, &raw).unwrap();
    assert!(BiomeRegions::read(dir.path()).is_err());
    assert_eq!(fs::read(&file).unwrap(), raw);
    assert_eq!(entries(file.parent().unwrap()), ["biome_regions.dat"]);
}

#[test]
fn literal_version_one_file_decodes_without_the_record_encoder() {
    let hex = concat!(
        "0a000003000776657273696f6e000000010a000a64696d656e73696f6e730a00136d696e6563726166743a6f76657277",
        "6f726c640a000873656c6563746f7203000776657273696f6e0000000103000463656c6c0000008008000473616c7400",
        "14686f72697a6f6e3a62696f6d655f726567696f6e03000e76616e696c6c615f7765696768740000000a000900077265",
        "67696f6e730a000000010800056f776e65720005616c70686108000269640009616c7068613a69636503000677656967",
        "687400000003090006706f696e74730a0000000108000562696f6d6500146d696e6563726166743a6963655f7370696b",
        "657309000b74656d70657261747572650500000002bf0000003e80000009000868756d69646974790500000002bf0000",
        "003e80000009000f636f6e74696e656e74616c6e6573730500000002bf0000003e80000009000765726f73696f6e0500",
        "000002bf0000003e80000009000564657074680500000002bf0000003e80000009000977656972646e65737305000000",
        "02bf0000003e8000000500066f66667365743ec000000000000000",
    );
    let bytes: Vec<u8> = hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect();
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(path(dir.path()).parent().unwrap()).unwrap();
    fs::write(path(dir.path()), &bytes).unwrap();
    let record = BiomeRegions::read(dir.path()).unwrap().unwrap();
    let layout = record.layout("minecraft:overworld").unwrap();
    assert_eq!(layout.selector, SelectorParams::current());
    assert_eq!(layout.regions[0].id, "alpha:ice");
    assert_eq!(layout.regions[0].weight, 3);
    assert_eq!(layout.regions[0].points[0].biome, "minecraft:ice_spikes");
    assert_eq!(layout.regions[0].points[0].intervals, [(-0.5, 0.25); 6]);
    assert_eq!(layout.regions[0].points[0].offset, 0.375);
    assert!(record.reported.is_empty());
    assert_eq!(fs::read(path(dir.path())).unwrap(), bytes);
}
