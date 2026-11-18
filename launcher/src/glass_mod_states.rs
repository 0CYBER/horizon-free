//! Debug-build previews of the native mod states a real failure or a real build makes, so each
//! can be screenshotted in the glass look: `HORIZON_LAUNCHER_PAGE=mod-…` (see [`NAMES`]). Every
//! text comes from the code that says it in a real build (`native_mods`), never from a copy.
//! Nothing here runs in a release build, and nothing is written to the player's data.
use super::*;
use build_mode::ModKind;
use horizon_launcher::native_mods::{
    self as nm,
    fit::{self, Verdict},
    sdk_cache::Refusal as Sdk,
    Route, Step,
};
use horizon_platform::mod_package::mod_surface::uses::{Change, Misfit};
use instance_mods::ModRow;

/// What a preview puts in place of what the launcher reads from disk.
#[derive(Default)]
pub(super) struct Preview {
    pub(super) active: bool,
    pub(super) routes: nm::Routes,
    pub(super) state: Option<native_status::State>,
    pub(super) update: Option<nm::UpdateCheck>,
    pub(super) left_out: Vec<nm::LeftOut>,
}

// Values: `mod-step-<key>`, `mod-error-<key>`, `mod-worlds[-many|-unbuilt]`, `mod-update-<all-fit|some|world|worlds>`,
// `mod-fit`, `mod-left-out`, `mod-trust[-generic|-registry]`, `mod-log[-failed]` (keys: see `step` and `error`).

/// Builds as `sdk_cache::shown` names them to players: as the Versions page does, no commit.
const BUILD: &str = "the Nov 13 update";
const MADE_FOR: &str = "the Nov 10 update";

/// The compiler's download, as the build card and the trust question say it.
fn compiler_bytes() -> u64 {
    horizon_launcher::rust_toolchain::host().map_or(0, |host| host.download_bytes())
}

fn misfits(count: usize) -> Vec<Misfit> {
    let ids = [
        ("hook:player_logged_in", Change::Changed),
        ("method:PlayersHost::player", Change::Removed),
        ("type:DimensionSettings", Change::Changed),
        ("fn:spawn_particles", Change::Removed),
        ("trait:ServerHost", Change::Changed),
        ("hook:block_broken", Change::Changed),
        ("method:ServerHost::give_item", Change::Removed),
    ];
    ids.into_iter()
        .take(count)
        .map(|(id, change)| Misfit {
            id: id.into(),
            change,
        })
        .collect()
}

fn unfit(name: &str, verdict: Verdict) -> Route {
    Route::Unfit {
        headline: fit::headline(name, &verdict),
        line: fit::line(name, &verdict, BUILD),
        actions: fit::actions(name),
    }
}

fn row(name: &str, kind: ModKind, error: Option<&str>) -> ModRow {
    ModRow {
        file: format!("mods/{name}.hmod"),
        name: name.into(),
        version: "1.2.0".into(),
        on: true,
        entry: None,
        icon: None,
        error: error.map(str::to_owned),
        missing: false,
        native: None,
        internals: Vec::new(),
        kind: Some((kind, name.into())),
    }
}

fn stake(worlds: &[&str], unbuilt: bool) -> nm::AtStake {
    let names: Vec<String> = worlds.iter().map(|w| (*w).to_owned()).collect();
    let text = match unbuilt {
        true => fit::worlds_at_stake_unbuilt("fast-ore", BUILD, &names),
        false => fit::worlds_at_stake("fast-ore", BUILD, &names),
    };
    nm::AtStake {
        text,
        worlds: names,
        mods: vec!["fast-ore".into()],
        unbuilt: if unbuilt {
            vec!["fast-ore".into()]
        } else {
            Vec::new()
        },
        backups: backend::data_dir().join("backups-preview"),
        stay: "nightly-2026-11-10".into(),
        base: "nightly-2026-11-13".into(),
    }
}

fn error(key: &str) -> Option<String> {
    let build = BUILD.to_owned();
    let sdk = |refusal: Sdk| refusal.to_string();
    Some(match key {
        "no-horizon" => tr("Install Horizon first.").to_string(),
        "no-sdk" => sdk(Sdk::NoSdk { build }),
        "offline-size" => sdk(Sdk::Offline {
            build,
            bytes: Some(212_000_000),
        }),
        "offline" => sdk(Sdk::Offline { build, bytes: None }),
        "unsigned" => sdk(Sdk::Unsigned {
            build,
            why: "checksum mismatch".into(),
        }),
        "launcher-schema" => sdk(Sdk::UpdateLauncher {
            build,
            what: trf("SDK schema {schema}", &[("schema", "9")]),
        }),
        "launcher-package" => sdk(Sdk::UpdateLauncher {
            build,
            what: trf("package format {format}", &[("format", "4")]),
        }),
        "launcher-rust" => sdk(Sdk::UpdateLauncher {
            build,
            what: trf("Rust {rust}", &[("rust", "1.99.0")]),
        }),
        "older-rust" => sdk(Sdk::OlderRust {
            build,
            rust: "1.80.0".into(),
        }),
        // A download that did not finish: the build says which file, and can run again.
        "retry" => sdk(Sdk::Refused {
            build,
            why: "sdk.json: connection reset".into(),
        }),
        "host" => sdk(Sdk::Host),
        "build-script" => nm::Refusal::BuildScript {
            mod_id: "fast-ore".into(),
        }
        .to_string(),
        "features" => nm::Refusal::Features {
            mod_id: "fast-ore".into(),
        }
        .to_string(),
        "state-layout" => trf(
            "E_MIXIN_STATE_LAYOUT: {mod}:{state}: native state changes the prebuilt game's layout; use an attachment instead",
            &[("mod", "fast-ore"), ("state", "counter")],
        ),
        "compile" => nm::failure(
            &nm::compiling_mod("fast-ore"),
            "exit status: 101",
            &["error[E0425]: cannot find value `speed` in this scope".into()],
            &["error: could not compile `fast-ore` (lib) due to 1 previous error".into()],
        ),
        "backup-failed" => trf(
            "Backing up {world} failed, so nothing was switched off or updated: {error}",
            &[("world", "Survival 1"), ("error", "disk full")],
        ),
        "cancelled" => tr(nm::CANCELLED).to_string(),
        "no-consent" => nm::Failed::NoConsent.to_string(),
        _ => return None,
    })
}

fn step(key: &str) -> Option<Step> {
    Some(match key {
        "start" => return None,
        "rust" => Step::Rust(0.42),
        "tool" => Step::Tool,
        "source" => Step::Source("afe471c0123456789abcdef0123456789abcdef0".into()),
        "mixins" => Step::Mixins,
        "cached-mixins" => Step::CachedMixins,
        "crates" => Step::Crates,
        "tools" => Step::Tools {
            build: BUILD.into(),
            done: 38_000_000,
            total: 112_000_000,
        },
        "tools-kept" => Step::ToolsKept {
            build: BUILD.into(),
        },
        "checking" => Step::Checking(3),
        "backed-up" => Step::BackedUp(trf(
            "Backed up {world} to {path}",
            &[("world", "Survival 1"), ("path", "backups/Survival 1.zip")],
        )),
        "plan" => Step::Plan(nm::Plan::Prebuilt, nm::Profile::Optimized),
        "mods" => Step::PrebuiltMods,
        "mod" => Step::Mod {
            id: "fast-ore".into(),
            done: 2,
            total: 3,
        },
        "install" => Step::Install,
        "left-out" => Step::LeftOut(left_out()[0].clone()),
        "done" => Step::Done {
            secs: 41,
            profile: nm::Profile::Optimized,
            plan: nm::Plan::Prebuilt,
        },
        _ => return None,
    })
}

fn left_out() -> Vec<nm::LeftOut> {
    vec![nm::LeftOut {
        id: "hitbox-plus".into(),
        version: "1.3.0".into(),
        why: nm::Why::Unfit("it changes the game's own code".into()),
    }]
}

impl Launcher {
    /// Applies the preview `HORIZON_LAUNCHER_PAGE` names, in debug builds only.
    pub(super) fn mod_preview_from_env(&mut self) {
        if !cfg!(debug_assertions) {
            return;
        }
        let Ok(page) = std::env::var("HORIZON_LAUNCHER_PAGE") else {
            return;
        };
        let Some(name) = page.strip_prefix("mod-") else {
            return;
        };
        let instance = instances::selected().id;
        self.mod_preview.active = true;
        self.native_for = Some(instance.clone());
        self.home_tab = home_tabs::HomeTab::Mods;
        self.instance_mods.fresh = true;
        self.instance_mods.list = vec![
            row("fast-ore", ModKind::Native, None),
            row("greeter", ModKind::Native, None),
            // Left out of a build, a mod stays installed and listed (`native_left_out`).
            row("hitbox-plus", ModKind::Native, None),
        ];
        self.mod_preview.state = Some(native_status::State::Outdated);
        let (kind, key) = name.split_once('-').unwrap_or((name, ""));
        // A build only ran after the player allowed native mods; the other states show the
        // row before that.
        if matches!(kind, "step" | "error") && key != "no-consent" {
            self.settings.native_mods_allowed_at = Some(1);
            *self.mods.native_state.borrow_mut() = Some((
                String::new(),
                native_status::State::Outdated,
                vec!["fast-ore".into(), "greeter".into()],
            ));
        }
        match (kind, key) {
            // How a build that worked looks once it ended: the step is cleared, the line says
            // how it went and the mods read as built (`Msg::Native`).
            ("step", "done") => {
                self.settings.native_mods_allowed_at = Some(1);
                self.mod_preview.state = Some(native_status::State::UpToDate);
                // Built: no mod waits for a build any more.
                *self.mods.native_state.borrow_mut() =
                    Some((String::new(), native_status::State::UpToDate, Vec::new()));
                self.native_done = step("done").map(|step| step.label());
            }
            ("step", "start") => {
                self.native_step = Some(tr("Starting the build").into());
                self.first_build = Some(compiler_bytes());
            }
            ("step", key) => {
                if let Some(step) = step(key) {
                    self.native_step = Some(step.shown());
                    self.native_progress = Some(step.progress());
                    if matches!(step, Step::Rust(_) | Step::Tools { .. }) {
                        self.first_build = Some(compiler_bytes());
                    }
                }
            }
            ("error", key) => {
                if key == "cancelled" {
                    self.native_done = error(key);
                } else if let Some(text) = error(key) {
                    // The same step a real failure takes.
                    self.native_failed(text, &["fast-ore".into(), "greeter".into()]);
                }
            }
            ("worlds", key) => {
                let (worlds, unbuilt): (&[&str], bool) = match key {
                    "many" => (&["Survival 1", "Creative test", "Hardcore"], false),
                    "unbuilt" => (&["Survival 1"], true),
                    _ => (&["Survival 1"], false),
                };
                let stake = stake(worlds, unbuilt);
                self.native_error = Some(stake.text.clone());
                self.keep_stop(&stake);
            }
            ("update", key) => {
                let to = "nightly-2026-11-13".to_owned();
                let off = [(
                    "hitbox-plus".to_owned(),
                    "it changes the game's own code".to_owned(),
                )];
                let (summary, worlds) = match key {
                    "some" => (fit::update_summary(BUILD, 3, &off), Vec::new()),
                    "world" => (
                        fit::update_summary(BUILD, 3, &off),
                        vec!["Survival 1".to_owned()],
                    ),
                    "worlds" => (
                        fit::update_summary(BUILD, 3, &off),
                        vec!["Survival 1".to_owned(), "Creative test".to_owned()],
                    ),
                    _ => (fit::update_summary(BUILD, 3, &[]), Vec::new()),
                };
                self.mod_preview.update = Some(nm::UpdateCheck {
                    to,
                    summary,
                    worlds,
                });
                self.mod_preview.state = Some(native_status::State::UpToDate);
            }
            ("fit", _) => {
                let pairs = [
                    (
                        "greeter",
                        Verdict::Fits {
                            made_for: MADE_FOR.into(),
                        },
                    ),
                    (
                        "hitbox-plus",
                        Verdict::Exact {
                            made_for: MADE_FOR.into(),
                            why: "native",
                        },
                    ),
                    (
                        "deep-tools",
                        Verdict::Exact {
                            made_for: MADE_FOR.into(),
                            why: "internals",
                        },
                    ),
                    (
                        "old-greeter",
                        Verdict::NoList {
                            made_for: MADE_FOR.into(),
                        },
                    ),
                    (
                        "night-vision",
                        Verdict::DoesNotFit {
                            made_for: MADE_FOR.into(),
                            misfits: misfits(1),
                        },
                    ),
                    (
                        "big-update",
                        Verdict::DoesNotFit {
                            made_for: MADE_FOR.into(),
                            misfits: misfits(7),
                        },
                    ),
                ];
                let mut mods = vec![("fast-ore".to_owned(), Route::Prebuilt)];
                let mut rows = vec![row("fast-ore", ModKind::Native, None)];
                for (name, verdict) in pairs {
                    let route = match &verdict {
                        Verdict::Fits { .. } => Route::Fits {
                            line: fit::line(name, &verdict, BUILD),
                        },
                        _ => unfit(name, verdict),
                    };
                    mods.push((name.to_owned(), route));
                    rows.push(row(name, ModKind::Native, None));
                }
                mods.push((
                    "scripted".into(),
                    Route::Invalid {
                        reason: error("build-script").unwrap_or_default(),
                    },
                ));
                rows.push(row("scripted", ModKind::Native, None));
                mods.push(("flaky".into(), Route::Prebuilt));
                rows.push(row("flaky", ModKind::Native, None));
                self.mods.unbuilt = Some((instance, vec!["flaky".into()]));
                rows.push(row("wasm-tweaks", ModKind::Wasm, None));
                rows.push(row(
                    "broken-wasm",
                    ModKind::Wasm,
                    Some("mod.wasm: not a WebAssembly module"),
                ));
                rows.push(row("pack-data", ModKind::Content, None));
                self.mod_preview.routes = nm::Routes {
                    mods,
                    unavailable: None,
                };
                self.instance_mods.list = rows;
            }
            ("left", _) => {
                // Left out by a build, which ran after the player allowed native mods.
                self.settings.native_mods_allowed_at = Some(1);
                self.mod_preview.left_out = left_out();
                // A build counts the mods it left out as tried (`native_status::built`): the
                // build is up to date, and only Switch off changes anything.
                self.mod_preview.state = Some(native_status::State::UpToDate);
                *self.mods.native_state.borrow_mut() =
                    Some((String::new(), native_status::State::UpToDate, Vec::new()));
            }
            ("trust", "registry") => {
                let (reply, _answer) = std::sync::mpsc::channel();
                let inventory = nm::RegistryInventory {
                    added_crates: 4,
                    build_scripts: vec!["libz-sys".into()],
                    proc_macros: vec!["serde_derive".into()],
                };
                let _ = self.tx.send(Msg::NativeRegistryConsent(inventory, reply));
            }
            ("trust", "generic") => {
                self.pending_consent = Some(PendingConsent::Build(Vec::new()));
            }
            ("trust", _) => {
                // Every switched-on native mod, as the real request names them.
                self.pending_consent = Some(PendingConsent::Build(vec![
                    "fast-ore".into(),
                    "greeter".into(),
                    "hitbox-plus".into(),
                ]));
            }
            ("log", key) => {
                // `mod-log-failed`: opened from a build that did not compile.
                if key == "failed" {
                    if let Some(text) = error("compile") {
                        self.native_failed(text, &["fast-ore".into()]);
                    }
                }
                self.native_log_open();
            }
            _ => {}
        }
    }
}
