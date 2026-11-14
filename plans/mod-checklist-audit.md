# Mod checklist audit (glass look, launcher-mod-checklist)

How the look is chosen: `theme.glass` (theme.rs:283 glass, :320 old blocky). Glass Home Mods tab =
`glass_home_mods.rs::glass_instance_mods`; glass Mods page (Page::Mods) = `glass_mods.rs`, which calls the
old `mods_page.rs::mods_installed_list` (glass_mods.rs:209). Both embed the shared card
`build_mode.rs::native_build` (glass palette via `Pal::of`, build_mode.rs:65). Old-only renderers:
`instance_mods.rs::mod_row/instance_mods_tab`, `build_mode.rs::route_chip`/`kind_badge`/`internals_chip`
(before this change), `instances_page.rs:232` instance sub-line.

Legend: REACHED = shared renderer already in glass; ADDED = this branch; PREVIEW = value below.

| # | State | Produced by (file:line, string) | Glass before | Now |
|---|---|---|---|---|
| 1 | Steps shown while running, Cancel | `native_mods.rs` `Step::label` (~1042): "Downloading Rust {version} (once): {done}%", "Building horizon-mixins (once)", "Downloading Horizon's source at {revision}", "Applying the mixins", "Reusing patched game source", "Checking native dependency pins and the game graph", "Getting the mod tools for {build}: {done} of {total} MB", "Checking {count} mods", "Compiling the mods against the prebuilt game", "Compiling {mod} ({done} of {total})", "Installing the build", "Built ({profile}, {plan})"; start `native_build.rs:44` "Starting the build"; pills `Stage::label` Prepare/Compile/Install; Cancel `build_mode.rs` building_card `native-build-cancel` | REACHED (Mods tab/page card; Home Play button progress + Cancel earlier) | PREVIEW `mod-step-*` |
| 2 | No SDK / offline (+size) / unsigned / launcher update (schema, package, Rust) / older Rust / refused or retryable download / host | `native_mods/sdk_cache.rs:66-107` (all strings), shown through `native_error` in `build_mode.rs::mode_status` | REACHED on Mods tab only; **Home head showed nothing after a failed Build and play** | ADDED `glass_build_notice` (glass_home_mods.rs) under Home head; PREVIEW `mod-error-*` |
| 2 | `E_NATIVE_BUILD_SCRIPT`, `E_NATIVE_FEATURES` | `native_mods/prebuilt.rs:37,41` | same | same; PREVIEW `mod-error-build-script/features` |
| 2 | `E_NATIVE_HOST` | `sdk_cache.rs:98` | same | PREVIEW `mod-error-host` |
| 2 | `E_MIXIN_STATE_LAYOUT` | `native_mods/package.rs:34` | same | PREVIEW `mod-error-state-layout` |
| 2 | SDK reused / downloading | `Step::Tools` label; first-build note `build_mode.rs` building_card ("First time only: ...") ; reuse is silent in code (no string; `sdk_cache::slices` keeps files) | REACHED | PREVIEW `mod-step-tools`; reuse has no wording to show (code gap, not glass) |
| 2 | Compile failure diagnostic, Show log | `native_mods.rs::failure` (now `pub`) ; `mode_status` "Show log" | REACHED | PREVIEW `mod-error-compile` |
| 3 | Trust notice, Rust download size | `blocks_settings.rs::native_consent` ("Allow native mods?", "{mod} is a native mod. ..."), root overlay `launcher_render.rs:327`; shown by Build / Build and play / switch on / install | REACHED (global modal, glass scrim) | PREVIEW `mod-trust`, `mod-trust-generic` |
| 3 | Dependency build scripts / proc macros permission | `native_registry_consent.rs` (window prompt: `Inventory::label/detail`) | REACHED (platform prompt, look independent) | PREVIEW `mod-trust-registry` |
| 3 | Allow / revoke in Settings, Storage | `glass_storage.rs:167` "Native mods have access to your computer." + Turn off | REACHED | — |
| 3 | Needs permission on a row | old `mods_page.rs:1529` badge + "Allow native mods" button | glass row had the word only | ADDED menu item "Allow native mods" in the status menu (`build_mode.rs::route_menu`) |
| 4 | Fit chips: fits, exact build only (table/internals), prebuilt, no list, does not fit (<=5 items, many variant), Needs attention | `build_mode.rs::route_facts` (was `route_chip`) over `native_mods/fit.rs` `line/headline/actions/changes`, `Route` from `planning.rs:27-110` | **missing** (glass row: word "Does not fit" only) | ADDED: the status word opens `route_menu` (headline, reason, Turn off, Build now, Show last log); PREVIEW `mod-fit` |
| 4 | Mod that doesn't fit stays installed | `fit::actions`, `LeftOut::label` (`isolate.rs:43`) | Mods page list only (`mods_page.rs:1658`) | ADDED left-out lines on Home Mods tab (`native_left_out`); PREVIEW `mod-left-out` |
| 5 | Worlds stop, names, stay (default) / back up first, failed backup, "did not build" | `native_mods.rs::guard_worlds` (~1680), `fit::worlds_at_stake*`, `build_mode.rs::worlds_choice`, `route_facts` didn't build | choice REACHED (card); did-not-build chip missing | ADDED via route menu; error on Home head; PREVIEW `mod-worlds`, `mod-worlds-many`, `mod-worlds-unbuilt`, `mod-error-backup-failed`, `mod-fit` (flaky row) |
| 6 | Update check (all fit, some, one world, worlds), Update / Stay | `build_mode.rs::update_card/update_panel`, `fit::update_summary/update_worlds` | REACHED | PREVIEW `mod-update-all-fit|some|world|worlds` |
| 7 | Build log page, progress, failure | `native_log_page.rs::native_log_panel` (root overlay, `launcher_render.rs:313`); opened from card, "Show log", row menu "View build log" | REACHED | PREVIEW `mod-log` |
| 8 | Unsigned developer base | `developer_install.rs:19 label`, `:34 instance_status` ("Local unsigned developer build · {commit} · {id} · Ready/Needs preparing"); old sidebar `instances_page.rs:232`; glass Versions/sheet via `version_label` | version_label REACHED; instance line missing | ADDED line under Home head (`glass_home.rs::played_text`); no preview (needs a developer-... version tag: `developer-install --trust-unsigned`) |
| 9 | Wasm next to native, kind, refusals | `ModKind::label` Native/WebAssembly/Content (`build_mode.rs`), `ModRow.error` | missing kind | ADDED kind pill (`kind_pill`); PREVIEW `mod-fit` (wasm-tweaks, broken-wasm error, pack-data). Wasm refusals at load happen in the game, not the launcher |
| 10 | Dark/light | `HORIZON_LAUNCHER_APPEARANCE=light|dark` (theme.rs:136) | — | combine with any preview |

## Previews (debug builds only)
`HORIZON_LAUNCHER_PAGE=` one of: `mod-step-{start,rust,tool,source,mixins,cached-mixins,crates,tools,checking,backed-up,plan,mods,mod,install,left-out,done}`,
`mod-error-{no-horizon,no-sdk,offline-size,offline,unsigned,launcher-schema,launcher-package,launcher-rust,older-rust,retry,host,build-script,features,state-layout,compile,backup-failed,cancelled,no-consent}`,
`mod-worlds`, `mod-worlds-many`, `mod-worlds-unbuilt`, `mod-update-{all-fit,some,world,worlds}`, `mod-fit`, `mod-left-out`,
`mod-trust`, `mod-trust-generic`, `mod-trust-registry`, `mod-log`. Code: `launcher/src/glass_mod_states.rs`; opens Home on the Mods tab with fake rows; texts come from the real constructors.

## Still missing
- "SDK files already kept (reused)": no string exists in `native_mods` (silent reuse).
- Fit wording says "under Native mod build" (`fit.rs:455`, `actions_pending`) while the card is titled "Mod build": left as is.
- Developer base preview needs a real `developer-…` version.
