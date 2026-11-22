use super::*;
use crate::native_mods::tests::{dir, write_bundle};
use std::rc::Rc;

const OTHER_BUILD: &str = "cccccccccccccccccccccccccccccccccccccccc";

/// A surface holding free functions (id, hash).
fn surface(functions: &[(&str, &str)]) -> mod_surface::Surface {
    let functions: Vec<_> = functions
        .iter()
        .map(|(id, hash)| {
            serde_json::json!({"id": id, "hash": hash, "signature": "fn f()", "wasm": true})
        })
        .collect();
    serde_json::from_value(serde_json::json!({
        "schema": mod_surface::SCHEMA, "hash": "s", "functions": functions,
    }))
    .unwrap()
}

/// A mod made for its bundle's pin whose table (when `uses` is given) lists those items.
fn project_dir(name: &str, uses: Option<&[(&str, &str)]>, made_for: Option<&str>) -> PathBuf {
    let dir = dir(name);
    write_bundle(&dir, "greeter", "");
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname='greeter'\nversion='0.1.0'\n[dependencies]\nhorizon-mod-api={ path = '.horizon/sdk/api/crates/horizon-mod-api' }\n",
    )
    .unwrap();
    let bundle = crate::native_mods::bundle_in(&dir).unwrap().unwrap();
    let pin = bundle.native().horizon.clone();
    if let Some(uses) = uses {
        let entries: Vec<_> = uses
            .iter()
            .map(|(id, hash)| serde_json::json!({"id": id, "hash": hash}))
            .collect();
        let table = serde_json::json!({
            "schema": uses::USES_SCHEMA,
            "made_for": made_for.unwrap_or(&pin),
            "entries": entries,
        });
        std::fs::write(dir.join(uses::USES_FILE), table.to_string()).unwrap();
    }
    dir
}

use std::path::{Path, PathBuf};

fn judged(dir: &Path, build: &str, target: &mod_surface::Surface) -> Verdict {
    let bundle = crate::native_mods::bundle_in(dir).unwrap().unwrap();
    let project = api::Project::read(&bundle).unwrap();
    let target = Rc::new(target.clone());
    judge(&project, build, &mut || Ok(target.clone())).unwrap()
}

fn pin(dir: &Path) -> String {
    let bundle = crate::native_mods::bundle_in(dir).unwrap().unwrap();
    bundle.native().horizon.clone()
}

#[test]
fn a_mod_fits_a_build_where_what_it_uses_is_unchanged() {
    let dir = project_dir("fit-fits", Some(&[("fn:give", "h1")]), None);
    // Something it doesn't use changed, and something new came: it still fits.
    let target = surface(&[("fn:give", "h1"), ("fn:other", "h9"), ("fn:new", "n")]);
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target),
        Verdict::Fits {
            made_for: pin(&dir)
        }
    );
    // On the build it was made for, no test runs.
    assert_eq!(judged(&dir, &pin(&dir), &surface(&[])), Verdict::MadeFor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_copy_a_build_makes_is_judged_by_the_same_table() {
    let dir = project_dir("fit-copied", Some(&[("fn:give", "h1")]), None);
    let bundle = crate::native_mods::bundle_in(&dir).unwrap().unwrap();
    let copy = crate::native_mods::tests::dir("fit-copied-to");
    crate::native_mods::package::copy(&bundle, &copy).unwrap();
    let target = surface(&[("fn:give", "h1")]);
    assert_eq!(
        judged(&copy, OTHER_BUILD, &target),
        Verdict::Fits {
            made_for: pin(&dir)
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
    std::fs::remove_dir_all(copy).unwrap();
}

#[test]
fn uses_table_changes_and_removal_invalidate_the_installed_build() {
    use crate::native_mods::{package, source_digest, version_for, Profile, Target};

    let dir = project_dir("fit-build-key", Some(&[("fn:give", "h1")]), None);
    let target = surface(&[("fn:give", "h2")]);
    let original = std::fs::read(dir.join(uses::USES_FILE)).unwrap();
    // The build hashes and judges its incoming copy, including requested mods it leaves out.
    let snapshot = |name: &str| {
        let copy = crate::native_mods::tests::dir(name);
        let bundle = crate::native_mods::bundle_in(&dir).unwrap().unwrap();
        package::copy(&bundle, &copy).unwrap();
        let copied = crate::native_mods::bundle_in(&copy).unwrap().unwrap();
        let digest = source_digest(&copied).unwrap();
        let version = version_for(
            OTHER_BUILD,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            Profile::Optimized,
            Target::Client,
            None,
            &[copied],
        )
        .unwrap();
        let verdict = judged(&copy, OTHER_BUILD, &target);
        std::fs::remove_dir_all(copy).unwrap();
        (version, digest, verdict)
    };
    let excluded = snapshot("fit-build-key-excluded");
    assert!(matches!(excluded.2, Verdict::DoesNotFit { .. }));
    assert_eq!(snapshot("fit-build-key-unchanged"), excluded);

    // Same manifest, sources and table length; only a compatibility fingerprint changes.
    let changed = String::from_utf8(original.clone())
        .unwrap()
        .replace("\"h1\"", "\"h2\"");
    std::fs::write(dir.join(uses::USES_FILE), changed).unwrap();
    let included = snapshot("fit-build-key-included");
    assert!(included.2.usable());
    assert_ne!(
        included.0, excluded.0,
        "a usable mod must not reuse the installed build's previous left_out decision"
    );
    assert_eq!(included.1, excluded.1, "compiler inputs did not change");
    assert_eq!(snapshot("fit-build-key-changed-stable"), included);

    std::fs::remove_file(dir.join(uses::USES_FILE)).unwrap();
    let absent = snapshot("fit-build-key-absent");
    assert!(matches!(absent.2, Verdict::NoList { .. }));
    assert_ne!(absent.0, included.0, "table presence decides inclusion");
    assert_ne!(absent.0, excluded.0, "a removed table is a different input");
    assert_eq!(absent.1, included.1);
    assert_eq!(snapshot("fit-build-key-absent-stable"), absent);

    // The same complete inputs select the same build, independent of the incoming path.
    std::fs::write(dir.join(uses::USES_FILE), original).unwrap();
    assert_eq!(snapshot("fit-build-key-restored"), excluded);
    let bundle = crate::native_mods::bundle_in(&dir).unwrap().unwrap();
    let tool = crate::native_mods::tests::dir("fit-build-key-tool");
    package::copy_tool_input(&bundle, &tool).unwrap();
    assert!(!tool.join(uses::USES_FILE).exists());
    assert_eq!(
        std::fs::read(tool.join("src/lib.rs")).unwrap(),
        std::fs::read(dir.join("src/lib.rs")).unwrap()
    );
    std::fs::remove_dir_all(tool).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_changed_or_removed_item_it_uses_is_named() {
    let dir = project_dir(
        "fit-misfit",
        Some(&[("fn:give", "h1"), ("hook:player_logged_in", "h2")]),
        None,
    );
    let target = surface(&[("fn:give", "h1-changed")]);
    let Verdict::DoesNotFit { made_for, misfits } = judged(&dir, OTHER_BUILD, &target) else {
        panic!("a changed item fits");
    };
    assert_eq!(made_for, pin(&dir));
    let said: Vec<String> = misfits.iter().map(change).collect();
    assert_eq!(
        said,
        [
            "the function give (changed)",
            "the hook player_logged_in (removed)"
        ]
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn without_a_list_for_its_pin_a_mod_works_on_its_own_build_only() {
    let none = project_dir("fit-no-list", None, None);
    let target = surface(&[("fn:give", "h1")]);
    assert_eq!(
        judged(&none, OTHER_BUILD, &target),
        Verdict::NoList {
            made_for: pin(&none)
        }
    );
    // A list made for another build than the mod's pin says nothing about this one.
    let stale = project_dir(
        "fit-stale-list",
        Some(&[("fn:give", "h1")]),
        Some("dddddddddddddddddddddddddddddddddddddddd"),
    );
    assert_eq!(
        judged(&stale, OTHER_BUILD, &target),
        Verdict::NoList {
            made_for: pin(&stale)
        }
    );
    // A list that does not read is no list: never a panic, never "fits".
    std::fs::write(
        none.join(uses::USES_FILE),
        b"{\"schema\": 1, \"made_for\": 3}",
    )
    .unwrap();
    assert_eq!(
        judged(&none, OTHER_BUILD, &target),
        Verdict::NoList {
            made_for: pin(&none)
        }
    );
    std::fs::remove_dir_all(none).unwrap();
    std::fs::remove_dir_all(stale).unwrap();
}

#[test]
fn a_mod_that_changes_game_code_is_exact_whatever_its_list_says() {
    let dir = project_dir("fit-exact", Some(&[("fn:give", "h1")]), None);
    let manifest = dir.join(crate::native_mods::NATIVE_MANIFEST);
    let text = std::fs::read_to_string(&manifest).unwrap()
        + "[[native.module]]\nid = \"m\"\nfile = \"game.rs\"\nname = \"m\"\nsource = \"src/m.rs\"\n";
    std::fs::write(&manifest, text).unwrap();
    std::fs::write(dir.join("src/m.rs"), "pub fn m() {}").unwrap();
    let target = surface(&[("fn:give", "h1")]);
    assert_eq!(
        judged(&dir, OTHER_BUILD, &target),
        Verdict::Exact {
            made_for: pin(&dir),
            why: "[[native.module]]"
        }
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn at_most_five_changes_are_named_then_how_many_more() {
    let misfits: Vec<Misfit> = (0..7)
        .map(|n| Misfit {
            id: format!("fn:f{n}"),
            change: Change::Removed,
        })
        .collect();
    let said = changes(&misfits);
    assert!(said.contains("f4"), "{said}");
    assert!(!said.contains("f5"), "{said}");
    assert!(
        said.ends_with("and 2 more (all of them are in the build log)"),
        "{said}"
    );
    assert_eq!(changes(&misfits[..1]), "the function f0 (removed)");
}

#[test]
fn every_message_names_the_mod_the_builds_and_the_worlds() {
    let made_for = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned();
    let misfit = Verdict::DoesNotFit {
        made_for: made_for.clone(),
        misfits: vec![
            Misfit {
                id: "hook:player_logged_in".into(),
                change: Change::Changed,
            },
            Misfit {
                id: "method:ServerHost::give_item".into(),
                change: Change::Removed,
            },
        ],
    };
    let build = "nightly-2026-11-13 (afe471c)";
    assert_eq!(
        refusal("Greeter", &misfit, build),
        "Greeter doesn't fit this game.\nIt was made for another Horizon version and this instance runs \
         nightly-2026-11-13 (afe471c). Since then the game changed 2 things that Greeter uses: the hook \
         player_logged_in (changed); the method ServerHost::give_item (removed).\nThe game will be \
         built without Greeter, which stays installed. Ask its author for an update made for this game."
    );
    let fits = Verdict::Fits {
        made_for: made_for.clone(),
    };
    assert_eq!(
        line("Greeter", &fits, build),
        "Made for another Horizon version. Fits this game too."
    );
    assert!(fits.usable() && Verdict::MadeFor.usable() && !misfit.usable());
    assert_eq!(
        worlds_at_stake(
            "Greeter",
            "nightly-2026-11-20 (9a8b7c6)",
            &["Home".into(), "Test island".into()]
        ),
        "Greeter doesn't fit nightly-2026-11-20 (9a8b7c6).\n2 of your worlds use Greeter: “Home” \
         and “Test island”. Opening them without Greeter removes its blocks and what they hold, \
         for good."
    );
    assert!(worlds_at_stake("Greeter", build, &["Home".into()]).contains("One of your worlds"));
}

#[test]
fn an_update_says_how_many_mods_fit_and_which_will_be_switched_off() {
    let build = "nightly-2026-11-20 (9a8b7c6)";
    assert_eq!(
        update_summary(build, 1, &[]),
        "Move to nightly-2026-11-20 (9a8b7c6)? Your native mod fits it."
    );
    assert_eq!(
        update_summary(build, 4, &[]),
        "Move to nightly-2026-11-20 (9a8b7c6)? All 4 of your native mods fit it."
    );
    let misfit = Verdict::DoesNotFit {
        made_for: OTHER_BUILD.into(),
        misfits: vec![Misfit {
            id: "hook:player_logged_in".into(),
            change: Change::Changed,
        }],
    };
    let off = [("Greeter".to_owned(), short(&misfit))];
    assert_eq!(
        update_summary(build, 4, &off),
        "Move to nightly-2026-11-20 (9a8b7c6)? 3 of your 4 native mods fit it.\nGreeter does \
         not: the hook player_logged_in (changed). It will be switched off until its author makes a version that fits."
    );
    let exact = Verdict::Exact {
        made_for: OTHER_BUILD.into(),
        why: "[[native.mixin]]",
    };
    assert_eq!(short(&exact), "it changes the game's own code");
}

#[test]
fn the_worlds_buttons_and_the_stopped_mod_say_what_is_so() {
    assert_eq!(
        back_up_and_build(1, 1, 0),
        "Back up this world and build without what doesn't fit"
    );
    assert_eq!(
        back_up_and_build(3, 1, 0),
        "Back up these worlds and build without what doesn't fit"
    );
    assert_eq!(
        back_up_and_build(1, 2, 2),
        "Back up this world and build without what didn't build"
    );
    assert_eq!(
        back_up_and_build(2, 2, 1),
        "Back up these worlds and build without those mods"
    );
    assert_eq!(back_up_and_update(1), "Back up this world and update");
    assert_eq!(back_up_and_update(2), "Back up these worlds and update");
    assert_eq!(
        actions_pending("Greeter"),
        "The build is stopped until you choose under Native mod build; Greeter stays on until then."
    );
    assert!(did_not_build_pending("Greeter").contains("stays on until you choose"));
    assert!(did_not_build_last("Greeter").contains("didn't build the last time"));
}
