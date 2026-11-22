//! Whether a native mod made for one game build fits another (SDK design §10): an API-only mod
//! (no `[native.*]` table, no internals) is built against any build where every item it uses is
//! unchanged, and only the built binary is exact to the build. The mod's uses table
//! ([`uses::USES_FILE`], written by `cargo horizon-mod build`) says what it uses, each item with
//! its fingerprint at the build it was made for; the build's `surface.json` says what is there
//! now; `mod_surface::uses::fits` compares the two. The compile against the build stays the final
//! check, and the loader's build-id check is unchanged.
//!
//! People read "made for" (a mod's pin) and "fits"; builds by their Nightly names; and items as
//! what they are ("the hook `player_logged_in`"), never surface ids or hashes.
use super::{api, prebuilt, sdk_cache, LeftOut, Route, Why};
use crate::i18n::{tr, trf};
use horizon_platform::mod_package::mod_surface;
use mod_surface::uses::{self, Change, Fit, ItemKind, Misfit, UsesTable};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

/// At most this many changed items are named in a message; the build log has them all.
pub const SHOWN_ITEMS: usize = 5;

/// How the mod's build compares to the instance's build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Age {
    Newer,
    Older,
    Unknown,
}

/// How a mod stands against the build an instance plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// Made for this build.
    MadeFor,
    /// Made for another build (`made_for`), and every item it uses is unchanged in this one.
    Fits { made_for: String },
    /// It changes game code or names the game's internals (`why`: the table that does, or
    /// `internals`): it works on the build it was made for only.
    Exact {
        made_for: String,
        why: &'static str,
        age: Age,
    },
    /// Its package doesn't say what it uses (made by an older tool, or its list is for another
    /// build than its pin): it works on the build it was made for only.
    NoList { made_for: String, age: Age },
    /// Items it uses changed in this build.
    DoesNotFit {
        made_for: String,
        misfits: Vec<Misfit>,
        age: Age,
    },
}

impl Verdict {
    /// Whether the mod can be built against this build.
    pub fn usable(&self) -> bool {
        matches!(self, Self::MadeFor | Self::Fits { .. })
    }

    /// The pinned build when it is known to be newer and the mod cannot run here.
    pub(super) fn newer_build(&self) -> Option<&str> {
        match self {
            Self::Exact {
                made_for,
                age: Age::Newer,
                ..
            }
            | Self::NoList {
                made_for,
                age: Age::Newer,
            }
            | Self::DoesNotFit {
                made_for,
                age: Age::Newer,
                ..
            } => Some(made_for),
            _ => None,
        }
    }
}

/// The uses table `dir` carries, when it carries one this launcher reads. A table that is too
/// large, of another schema or malformed is no table: the mod then works on its own build only.
pub(super) fn table(dir: &Path) -> Option<UsesTable> {
    let file = std::fs::File::open(dir.join(uses::USES_FILE)).ok()?;
    let mut bytes = Vec::new();
    file.take(uses::MAX_USES_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    UsesTable::read(&bytes).ok()
}

/// How `project` stands against build `build`, whose surface `surface` gives (asked for only when
/// the mod was made for another build).
pub(super) fn judge(
    project: &api::Project<'_>,
    build: &str,
    surface: &mut dyn FnMut() -> Result<std::rc::Rc<mod_surface::Surface>, sdk_cache::Refusal>,
) -> Result<Verdict, sdk_cache::Refusal> {
    let made_for = project.bundle.native().horizon.clone();
    if made_for == build {
        return Ok(Verdict::MadeFor);
    }
    if let Some(why) = prebuilt::changes_game(project) {
        let age = sdk_cache::age(&made_for, build);
        return Ok(Verdict::Exact { made_for, why, age });
    }
    let Some(table) = table(&project.bundle.dir).filter(|table| table.made_for == made_for) else {
        let age = sdk_cache::age(&made_for, build);
        return Ok(Verdict::NoList { made_for, age });
    };
    let target = surface()?;
    if let Fit::DoesNotFit(misfits) = uses::fits(&table.entries, target.as_ref()) {
        let age = sdk_cache::age(&made_for, build);
        return Ok(Verdict::DoesNotFit {
            made_for,
            misfits,
            age,
        });
    }
    Ok(Verdict::Fits { made_for })
}

/// An item as people read it: "the hook `player_logged_in`".
pub fn item(id: &str) -> String {
    let Some((kind, name)) = uses::item(id) else {
        return id.to_owned();
    };
    let en = match kind {
        ItemKind::Hook => "the hook {name}",
        ItemKind::Argument => "the argument {name}",
        ItemKind::Trait => "the trait {name}",
        ItemKind::Method => "the method {name}",
        ItemKind::AssociatedItem => "the associated item {name}",
        ItemKind::Function => "the function {name}",
        ItemKind::Macro => "the macro {name}",
        ItemKind::PartKind => "the Part kind {name}",
        ItemKind::Capability => "the capability {name}",
        ItemKind::Type => "the type {name}",
        ItemKind::Variant => "the variant {name}",
        ItemKind::Field => "the field {name}",
    };
    trf(en, &[("name", &name)])
}

/// What changed about one item, as people read it: "the hook `player_logged_in` (changed)".
pub fn change(misfit: &Misfit) -> String {
    let what = item(&misfit.id);
    let leaf = |id: &str| mod_surface::leaf(id).to_owned();
    match &misfit.change {
        Change::Removed => trf("{item} (removed)", &[("item", &what)]),
        Change::Changed => trf("{item} (changed)", &[("item", &what)]),
        Change::LostImpl(name) => trf(
            "{item} no longer implements {trait}",
            &[("item", &what), ("trait", name)],
        ),
        Change::Fields { gained, at } if gained.is_empty() => trf(
            "the fields of {item} changed, and {at} builds it field by field",
            &[("item", &what), ("at", at)],
        ),
        Change::Fields { gained, at } => trf(
            "{item} gained {fields}, and {at} builds it field by field",
            &[
                ("item", &what),
                ("fields", &list(gained.iter().cloned())),
                ("at", at),
            ],
        ),
        Change::Version { named, now } => trf(
            "{item} is now version {now}; this mod names version {named}",
            &[
                ("item", &what),
                ("now", &now.to_string()),
                ("named", &named.to_string()),
            ],
        ),
        Change::NotBuildable { at } => trf(
            "{item} can no longer be built with a literal, and {at} builds it",
            &[("item", &what), ("at", at)],
        ),
        Change::Path(path) => trf(
            "mod_api::{path} is no longer exported under that path",
            &[("path", path)],
        ),
        Change::Route { from, owner } => trf(
            "{from} no longer includes {owner}, which this mod calls {member} on",
            &[
                ("from", &leaf(from)),
                ("owner", &leaf(owner)),
                ("member", &leaf(&misfit.id)),
            ],
        ),
    }
}

/// Words joined as a list: "a", "a and b", "a, b and c".
fn list(words: impl Iterator<Item = String>) -> String {
    let words: Vec<String> = words.collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => trf(
            "{first} and {last}",
            &[("first", &rest.join(", ")), ("last", last)],
        ),
    }
}

/// The changed items of `misfits`, at most [`SHOWN_ITEMS`], then how many more.
pub fn changes(misfits: &[Misfit]) -> String {
    let shown: Vec<String> = misfits.iter().take(SHOWN_ITEMS).map(change).collect();
    let more = misfits.len().saturating_sub(SHOWN_ITEMS);
    let shown = shown.join("; ");
    match more {
        0 => shown,
        more => trf(
            "{items}; and {count} more (all of them are in the build log)",
            &[("items", &shown), ("count", &more.to_string())],
        ),
    }
}

/// The line a mod's row shows about `verdict` for mod `name` in an instance on `build`
/// (as people read it).
pub fn line(name: &str, verdict: &Verdict, build: &str) -> String {
    let made = |commit: &str| sdk_cache::known_name(commit);
    match verdict {
        Verdict::MadeFor => tr("Made for this game.").to_string(),
        Verdict::Fits { made_for } => trf(
            "Made for {made_for}. Fits this game too.",
            &[("made_for", &made(made_for))],
        ),
        Verdict::Exact { made_for, why, .. } if *why == "internals" => trf(
            "Made for {made_for} only: it uses the game's internals, so it works on exactly that build.",
            &[("made_for", &made(made_for))],
        ),
        Verdict::Exact { made_for, .. } => trf(
            "Made for {made_for} only: it changes the game's own code, so it works on exactly that build.",
            &[("made_for", &made(made_for))],
        ),
        Verdict::NoList { made_for, age: Age::Newer } => trf(
            "{mod}'s package doesn't say what it uses, so it works on {made_for} only.",
            &[("mod", name), ("made_for", &made(made_for))],
        ),
        Verdict::NoList { made_for, .. } => trf(
            "{mod}'s package was made by an older tool and doesn't say what it uses, so it works on {made_for} only. Its author can rebuild it with the current mod tools.",
            &[("mod", name), ("made_for", &made(made_for))],
        ),
        Verdict::DoesNotFit { made_for, misfits, age } => {
            let made_for = made(made_for);
            let items = changes(misfits);
            let count = misfits.len().to_string();
            let args = [
                ("made_for", made_for.as_str()),
                ("build", build),
                ("count", count.as_str()),
                ("mod", name),
                ("items", items.as_str()),
            ];
            match (age, misfits.len()) {
                (Age::Newer, 1) => trf(
                    "It was made for {made_for}, which is newer than {build}, the version this instance runs. One thing {mod} uses is different there: {items}.",
                    &args,
                ),
                (Age::Newer, _) => trf(
                    "It was made for {made_for}, which is newer than {build}, the version this instance runs. {count} things {mod} uses are different there: {items}.",
                    &args,
                ),
                (_, 1) => trf(
                    "It was made for {made_for} and this instance runs {build}. Since then the game changed one thing that {mod} uses: {items}.",
                    &args,
                ),
                (_, _) => trf(
                    "It was made for {made_for} and this instance runs {build}. Since then the game changed {count} things that {mod} uses: {items}.",
                    &args,
                ),
            }
        }
    }
}

/// The headline over a verdict the build can't use: "Greeter doesn't fit this game."
pub fn headline(name: &str, verdict: &Verdict) -> String {
    match verdict {
        Verdict::MadeFor | Verdict::Fits { .. } => trf("{mod} fits this game.", &[("mod", name)]),
        Verdict::Exact { .. } | Verdict::NoList { .. } => {
            trf("{mod} works on another version only.", &[("mod", name)])
        }
        Verdict::DoesNotFit { .. } => trf("{mod} doesn't fit this game.", &[("mod", name)]),
    }
}

/// The status and guidance for a mod that cannot be built against this game.
pub fn unfit_route(name: &str, verdict: &Verdict, build: &str) -> Route {
    let direction = verdict.newer_build().map(update_direction);
    Route::Unfit {
        headline: headline(name, verdict),
        line: line(name, verdict, build),
        actions: actions(name, direction.as_deref()),
        pending_actions: actions_pending(name, direction.as_deref()),
    }
}

fn update_direction(made_for: &str) -> String {
    trf(
        "Update this instance to {made_for} to use it: instance settings, Installation, Game version.",
        &[("made_for", &sdk_cache::known_name(made_for))],
    )
}

/// What the player can do about a mod the build can't use.
fn actions(name: &str, direction: Option<&str>) -> String {
    if let Some(direction) = direction {
        let omitted = trf(
            "The game will be built without {mod}, which stays installed.",
            &[("mod", name)],
        );
        return format!("{omitted} {direction}");
    }
    trf(
        "The game will be built without {mod}, which stays installed. Ask its author for an update made for this game.",
        &[("mod", name)],
    )
}

/// Why a mod the build can't use won't be built, in a few words: what changed, or why it is
/// tied to its own build.
fn short(verdict: &Verdict) -> String {
    match verdict {
        Verdict::MadeFor | Verdict::Fits { .. } => String::new(),
        Verdict::Exact { why, .. } if *why == "internals" => {
            tr("it uses the game's internals").to_owned()
        }
        Verdict::Exact { .. } => tr("it changes the game's own code").to_owned(),
        Verdict::NoList { .. } => tr("its package doesn't say what it uses").to_owned(),
        Verdict::DoesNotFit { misfits, .. } => changes(misfits),
    }
}

/// Why an offered update would switch a mod off.
pub(super) fn update_why(verdict: &Verdict) -> Why {
    let why = short(verdict);
    if verdict.newer_build().is_some() {
        Why::Newer(why)
    } else {
        Why::Unfit(why)
    }
}

/// What updating to `build` does to the instance's `total` native mods, said before anything is
/// downloaded (design-fit §10.1): how many fit, and each one that will be switched off.
pub fn update_summary(build: &str, total: usize, off: &[LeftOut]) -> String {
    if off.is_empty() {
        return if total == 1 {
            trf(
                "Move to {build}? Your native mod fits it.",
                &[("build", build)],
            )
        } else {
            trf(
                "Move to {build}? All {count} of your native mods fit it.",
                &[("build", build), ("count", &total.to_string())],
            )
        };
    }
    let head = trf(
        "Move to {build}? {fit} of your {count} native mods fit it.",
        &[
            ("build", build),
            ("fit", &total.saturating_sub(off.len()).to_string()),
            ("count", &total.to_string()),
        ],
    );
    let lines: Vec<String> = off
        .iter()
        .map(|left| {
            let name = &left.id;
            match &left.why {
                Why::Newer(why) => trf(
                    "{mod} does not: {why}. It will be switched off. To use it, update this instance to the version it was made for.",
                    &[("mod", name), ("why", why.trim_end_matches('.'))],
                ),
                Why::Unfit(why) => update_off(name, why),
                Why::Needs(library) => {
                    let why = trf("it needs {library}, which doesn't fit", &[("library", library)]);
                    match newer_requirement(library, off) {
                        Some(root) => trf(
                            "{mod} does not: {why}. It will be switched off. {library} was made for a newer game build than this update offers.",
                            &[("mod", name), ("why", &why), ("library", root)],
                        ),
                        None => update_off(name, &why),
                    }
                }
                _ => left.label(),
            }
        })
        .collect();
    format!("{head}\n{}", lines.join("\n"))
}

/// Find the newer-build exclusion at the end of a dependency chain, if there is one.
fn newer_requirement<'a>(mut library: &'a str, off: &'a [LeftOut]) -> Option<&'a str> {
    let mut visited = HashSet::new();
    while visited.insert(library) {
        let left = off.iter().find(|left| left.id == library)?;
        match &left.why {
            Why::Needs(next) => library = next,
            Why::Newer(_) => return Some(&left.id),
            _ => return None,
        }
    }
    None
}

fn update_off(name: &str, why: &str) -> String {
    trf(
        "{mod} does not: {why}. It will be switched off until its author makes a version that fits.",
        &[("mod", name), ("why", why.trim_end_matches('.'))],
    )
}

/// Said under an update's summary when worlds were saved with a mod it switches off.
pub fn update_worlds(worlds: &[String]) -> String {
    let named = list(worlds.iter().map(|world| format!("“{world}”")));
    if worlds.len() == 1 {
        trf(
            "One of your worlds uses a mod this update switches off: {worlds}. Opening it without that mod removes its blocks and what they hold, for good.",
            &[("worlds", &named)],
        )
    } else {
        trf(
            "{count} of your worlds use a mod this update switches off: {worlds}. Opening them without it removes its blocks and what they hold, for good.",
            &[("count", &worlds.len().to_string()), ("worlds", &named)],
        )
    }
}

/// The button that takes the update.
pub fn update() -> &'static str {
    tr("Update")
}

/// The whole message for mod `name` that the build can't use: headline, why, and what to do.
pub fn refusal(name: &str, verdict: &Verdict, build: &str) -> String {
    format!(
        "{}\n{}\n{}",
        headline(name, verdict),
        line(name, verdict, build),
        actions(name, verdict.newer_build().map(update_direction).as_deref())
    )
}

/// The message after a mod judged to fit didn't compile.
pub fn did_not_build(name: &str) -> String {
    trf(
        "{mod} looked compatible with this game but didn't build. This is for {mod}'s author to fix; the mod is switched off here. Details were saved to the build log.",
        &[("mod", name)],
    )
}

/// What the card says when a mod's own compile failed: no compiler text, the way on.
pub fn compile_failed(name: &str) -> String {
    trf(
        "{mod} couldn't be built. The problem is in the mod, not in your game. Switch it off to play, or send the log to its author.",
        &[("mod", name)],
    )
}

/// The mod's menu while the stop for worlds saved with it is on screen: nothing is switched off
/// until the player chooses.
pub fn did_not_build_pending(name: &str) -> String {
    trf(
        "{mod} looked compatible with this game but didn't build. It stays on until you choose under Native mod build; this is for {mod}'s author to fix.",
        &[("mod", name)],
    )
}

/// What is said when a mod the build can't use was saved into worlds: the worlds named, and
/// that opening them without the mod removes its content for good. `build` is the build the
/// mod doesn't fit.
pub fn worlds_at_stake(name: &str, build: &str, worlds: &[String]) -> String {
    let headline = trf(
        "{mod} doesn't fit {build}.",
        &[("mod", name), ("build", build)],
    );
    format!("{headline}\n{}", worlds_body(name, worlds))
}

/// [`worlds_at_stake`] for a mod made for a newer build.
pub(super) fn worlds_at_stake_newer(
    name: &str,
    build: &str,
    worlds: &[String],
    made_for: &str,
) -> String {
    format!(
        "{}\n{}",
        worlds_at_stake(name, build, worlds),
        update_direction(made_for)
    )
}

/// [`worlds_at_stake`] for a mod judged to fit `build` whose compile against it failed.
pub fn worlds_at_stake_unbuilt(name: &str, build: &str, worlds: &[String]) -> String {
    let headline = trf(
        "{mod} looked compatible with {build} but didn't build.",
        &[("mod", name), ("build", build)],
    );
    format!("{headline}\n{}", worlds_body(name, worlds))
}

/// The worlds saved with mod `name`, and what opening them without it does.
fn worlds_body(name: &str, worlds: &[String]) -> String {
    let named = list(worlds.iter().map(|world| format!("“{world}”")));
    if worlds.len() == 1 {
        trf(
            "One of your worlds uses {mod}: {worlds}. Opening it without {mod} removes its blocks and what they hold, for good.",
            &[("mod", name), ("worlds", &named)],
        )
    } else {
        trf(
            "{count} of your worlds use {mod}: {worlds}. Opening them without {mod} removes its blocks and what they hold, for good.",
            &[
                ("count", &worlds.len().to_string()),
                ("mod", name),
                ("worlds", &named),
            ],
        )
    }
}

/// The choice that keeps everything as it is, said on its button: the version `current` (a
/// game build's tag) named as the Versions page names it.
pub fn stay(current: &str) -> String {
    let name = crate::nightly::update_name(current).unwrap_or_else(|| current.to_owned());
    trf("Stay on {build}", &[("build", &name)])
}

/// The other choice, said on its button.
pub fn back_up_and_update(worlds: usize) -> &'static str {
    match worlds {
        1 => tr("Back up this world and update"),
        _ => tr("Back up these worlds and update"),
    }
}

/// [`back_up_and_update`] on the Build panel, where nothing is being updated.
pub fn back_up_and_build(worlds: usize, mods: usize, unbuilt: usize) -> &'static str {
    match (worlds, unbuilt) {
        (1, 0) => tr("Back up this world and build without what doesn't fit"),
        (_, 0) => tr("Back up these worlds and build without what doesn't fit"),
        (1, n) if n == mods => tr("Back up this world and build without what didn't build"),
        (_, n) if n == mods => tr("Back up these worlds and build without what didn't build"),
        (1, _) => tr("Back up this world and build without those mods"),
        (_, _) => tr("Back up these worlds and build without those mods"),
    }
}

/// The choice on the Build panel that builds nothing.
pub fn dont_build() -> &'static str {
    tr("Don't build; keep the game as it is")
}

/// The chip's headline for a mod whose compile failed.
pub fn didnt_build_headline(name: &str) -> String {
    trf("{mod} didn't build", &[("mod", name)])
}

/// The chip's menu after the player chose and the failed mod is still there.
pub fn did_not_build_last(name: &str) -> String {
    trf(
        "{mod} looked compatible with this game but didn't build the last time. This is for {mod}'s author to fix.",
        &[("mod", name)],
    )
}

/// What the mod's status says while a world prevents its omission.
fn actions_pending(name: &str, direction: Option<&str>) -> String {
    let pending = trf(
        "The build is stopped until you choose under Native mod build; {mod} stays on until then.",
        &[("mod", name)],
    );
    match direction {
        Some(direction) => format!("{pending} {direction}"),
        None => pending,
    }
}

#[cfg(test)]
#[path = "fit_tests.rs"]
mod tests;
