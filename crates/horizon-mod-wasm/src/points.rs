//! Points use the hook transport and register in the native points list.
use crate::{CallClass, Refusal, WasmMod, WeakWasmMod, boundary::Frame};
use horizon_mod_package::manifest::{
    Manifest,
    inject::{Inject, Kind},
};
use mod_points::__private::{Forward, PointHost, SlotListener};
use mod_points::PointId;
use std::cell::RefCell;

struct Endpoint {
    owner: &'static str,
    module: WeakWasmMod,
    listener: u32,
}

impl Forward for Endpoint {
    fn forward(&self, id: u32, host: PointHost<'_>, record: &mut dyn mod_api::wire::Record) {
        let mut frame = Frame::default();
        let mut side = None;
        match host {
            PointHost::Read(host) => frame.set_host(host),
            PointHost::Server(host) => frame.set_server_host_mut(host),
            PointHost::Client(host) => frame.set_client_host_mut(host),
            PointHost::Side(mod_api::HostSide::Server(host)) => {
                side = Some(true);
                frame.set_server_host_mut(host);
            }
            PointHost::Side(mod_api::HostSide::Client(host)) => {
                side = Some(false);
                frame.set_client_host_mut(host);
            }
            PointHost::None => {}
        }
        self.invoke(
            frame,
            id,
            |out| {
                if let Some(server) = side {
                    out.put(&server);
                }
            },
            record,
        );
    }
}

impl Endpoint {
    fn invoke(
        &self,
        frame: Frame,
        id: u32,
        prefix: impl FnOnce(&mut mod_api::wire::Writer),
        record: &mut dyn mod_api::wire::Record,
    ) {
        let record = RefCell::new(record);
        self.module.call(
            CallClass::Hook,
            frame,
            id,
            None,
            |out| {
                out.put(&self.listener);
                prefix(out);
                record.borrow().encode(out);
            },
            |input| {
                input.take::<()>()?;
                let bytes = input.take_bytes(input.rest().len())?;
                record.borrow_mut().apply(bytes)
            },
        );
        if self.module.is_stopped() {
            mod_points::stop(self.owner);
        }
    }
}

/// Check engine subscriptions completely before registering in the host's list.
pub(crate) fn listen(manifest: &Manifest, module: &WasmMod) -> Result<(), Refusal> {
    listen_checked(manifest, module).map_err(Refusal::Runtime)
}

fn listen_checked(manifest: &Manifest, module: &WasmMod) -> Result<(), String> {
    let engine: Vec<_> = manifest.inject.iter().collect();
    if engine.is_empty() {
        return Ok(());
    }
    // The table's own answer, before the guest is asked anything: `register` would refuse each.
    if !mod_points::__private::TABLE.has_sites() {
        return Err(format!(
            "{}: {}: this build has no point sites; start the game through the launcher's mod build",
            manifest.id, engine[0].at
        ));
    }
    for inject in &engine {
        if let Some((_, reason)) = mod_points::API_REFUSALS.iter().find(|(target, _)| {
            horizon_mod_point_emit::plain_path(target)
                == horizon_mod_point_emit::plain_path(&inject.at)
        }) {
            return Err(format!(
                "{}:{}: no API listener at {}: {reason}",
                manifest.id, inject.id, inject.at
            ));
        }
    }
    let listed: mod_points::Subscriptions = module
        .call(
            CallClass::Load,
            Frame::default(),
            mod_api::wire::POINT_SUBSCRIPTIONS,
            None,
            |_| {},
            |input| input.take(),
        )
        .ok_or("point subscriptions did not answer")?;
    let mut declared = engine
        .into_iter()
        .map(|inject| (inject.id.as_str(), inject))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut slots = Vec::new();
    for (index, (place, name, identity, id)) in listed.into_iter().enumerate() {
        let inject = declared
            .remove(id.as_str())
            .ok_or_else(|| format!("undeclared point listener {id}"))?;
        // By name: the guest's place ids are its own build's, and may differ from the game's.
        let point = mod_points::point_by_name(&name).ok_or_else(|| {
            format!(
                "{}: {id}: listens at point {name}, which this game doesn't have: it was built for an older or newer game and needs a build for this one",
                manifest.id
            )
        })?;
        if mod_points::point_identity(point).1 != identity {
            return Err(format!(
                "{}: {id}: built against another version of point {name}, which this game changed: rebuild it",
                manifest.id
            ));
        }
        checked_place(&id, inject, point).map_err(|e| format!("{}: {e}", manifest.id))?;
        // Checked before any listener is registered.
        let listener_id = listener_id(place).ok_or_else(|| {
            format!(
                "{}: {id}: point index {place} is outside the points' range",
                manifest.id
            )
        })?;
        slots.push((
            point,
            listener_id,
            index as u32,
            inject.priority,
            id,
            inject.kind == Kind::Replace,
        ));
    }
    if let Some(id) = declared.keys().next() {
        return Err(format!("unfilled point listener {id}"));
    }
    let owner = Box::leak(manifest.id.clone().into_boxed_str());
    for (place, listener_id, listener, priority, id, replace) in slots {
        let endpoint = Box::leak(Box::new(Endpoint {
            owner,
            module: module.downgrade(),
            listener,
        }));
        let id = Box::leak(id.into_boxed_str());
        let mut slot = SlotListener::forwarded(owner, place.place(), endpoint, listener_id)
            .ordered(priority, id);
        // The guest's listener already owns the outcome (`mod_points::replace!`); the host's
        // slot carries the mark, so a second replace at the place is refused for either target.
        if replace {
            slot = slot.replacing();
        }
        mod_points::register(&slot).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The listener id of the mod's own point index `place`: in the points' range, below the
/// subscriptions id. `None` for an index the mod made up.
fn listener_id(place: u32) -> Option<u32> {
    mod_api::wire::POINTS
        .checked_add(place)
        .filter(|id| *id < mod_api::wire::POINT_SUBSCRIPTIONS)
}

/// The engine's API place `place` that a guest subscribed as its declared `inject`, checked
/// against the game's own point list, not what the module says of itself: the target, the phase,
/// and, for a `replace`, that its head returns `()` or an API value the listener can produce.
fn checked_place(id: &str, inject: &Inject, place: PointId) -> Result<(), String> {
    let Some(&(_, target, at_return, replace_refusal)) = mod_points::API_PLACES
        .iter()
        .find(|(point, ..)| *point == place)
    else {
        return Err(format!(
            "{id}: {}: no API record for place {place:?}",
            inject.at
        ));
    };
    if (inject.kind == Kind::Return) != at_return
        || horizon_mod_point_emit::plain_path(&inject.at)
            != horizon_mod_point_emit::plain_path(target)
    {
        return Err(format!(
            "{id}: listener does not match its declared API head/return target"
        ));
    }
    if let Some(ty) = replace_refusal.filter(|_| inject.kind == Kind::Replace) {
        return Err(format!(
            "{id}: no replace listener at {target}: a replace returns the function's value, and \
             `{ty}` has no API form"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inject(kind: Kind, at: &str) -> Inject {
        Inject {
            id: "own".into(),
            at: at.into(),
            kind,
            with: "own".into(),
            priority: 0,
            hash: Some("pin".into()),
        }
    }

    #[test]
    #[cfg(not(feature = "listener-fixture"))]
    fn unstaged_build_refuses_api_listener_by_mod_and_target() {
        crate::test_stack::on_guest_stack(|| {
            const { assert!(!mod_points::STAGED) };
            let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "site_test", "name": "Sites", "version": "1.0.0", "description": "", "api": "0.2", "capabilities": [],
            "inject": [{"id": "head", "at": "game::tick", "kind": "head", "with": "listener"}]
        }))
        .unwrap();
            let bytes = wat::parse_str(format!(
                r#"(module
            (@custom "dylink.0" "\01\04\00\00\00\00")
            (import "env" "memory" (memory 1))
            (func (export "horizon_layout") (result i64) i64.const {})
            (func (export "horizon_in") (param i32) (result i32) i32.const 0)
            (func (export "horizon_hook") (param i32) (result i64) {}))"#,
                *mod_api::wire::LAYOUT,
                crate::calls::answering::<&str>(&[], "i64.const 0")
            ))
            .unwrap();
            let mut modules = WasmMod::instantiate(
                &crate::runtime::engine().unwrap(),
                crate::runtime::DEFAULT_GROUP_MEMORY_MIB,
                &[crate::link::Member {
                    name: "libsite.so",
                    module: &bytes,
                }],
                None,
            )
            .unwrap();
            assert_eq!(
                listen_checked(&manifest, &modules.remove(0)).unwrap_err(),
                "site_test: game::tick: this build has no point sites; start the game through the launcher's mod build"
            );
        })
    }

    #[test]
    #[cfg(feature = "listener-fixture")]
    fn wasm_loader_refuses_no_wire_by_mod_target_and_type() {
        crate::test_stack::on_guest_stack(|| {
            let manifest = Manifest::parse(&format!(
                r#"id='refused_fixture'
name='Refused fixture'
version='1.0.0'
api='0.2'
capabilities=['registries']
[wasm]
sha512='{}'
toolchain='fixture'
[[inject]]
id='no_wire'
at='listener_fixture::no_wire'
kind='head'
with='no_wire'
"#,
                "0".repeat(128)
            ))
            .unwrap();
            let bytes = wat::parse_str(format!(
                r#"(module
            (@custom "dylink.0" "\01\04\00\00\00\00")
            (import "env" "memory" (memory 1))
            (func (export "horizon_layout") (result i64) i64.const {})
            (func (export "horizon_in") (param i32) (result i32) i32.const 0)
            (func (export "horizon_hook") (param i32) (result i64) {}))"#,
                *mod_api::wire::LAYOUT,
                crate::calls::answering::<&str>(&[], "i64.const 0")
            ))
            .unwrap();
            let mut modules = WasmMod::instantiate(
                &crate::runtime::engine().unwrap(),
                crate::runtime::DEFAULT_GROUP_MEMORY_MIB,
                &[crate::link::Member {
                    name: "refused.so",
                    module: &bytes,
                }],
                None,
            )
            .unwrap();
            let error = listen(&manifest, &modules.remove(0))
                .unwrap_err()
                .to_string();
            assert_eq!(
                error,
                "refused_fixture:no_wire: no API listener at listener_fixture::no_wire: argument 1 `value` (`&RegistrationData`) has no API type yet"
            );
        })
    }

    /// A point index from the mod is used only inside the points' range: never the
    /// subscriptions id, never past the range, and never by overflowing.
    #[test]
    fn a_point_index_outside_the_range_is_refused() {
        assert_eq!(listener_id(0), Some(mod_api::wire::POINTS));
        let last = mod_api::wire::POINT_SUBSCRIPTIONS - mod_api::wire::POINTS;
        assert_eq!(
            listener_id(last - 1),
            Some(mod_api::wire::POINT_SUBSCRIPTIONS - 1)
        );
        for place in [last, last + 1, 0x7FFF_FFFF, u32::MAX] {
            assert_eq!(listener_id(place), None, "{place:#x}");
        }
    }

    /// A module built without the build tool may subscribe a `replace` anywhere: the loader
    /// refuses one at a head whose return it can't produce, by the game's own point list.
    #[test]
    fn a_replace_is_checked_against_the_games_point_list_at_load() {
        let heads = || {
            mod_points::API_PLACES
                .iter()
                .filter(|(_, _, at_return, _)| !at_return)
        };
        let &(fallible, target, _, refusal) = heads()
            .find(|(.., refusal)| refusal.is_some_and(|ty| ty.starts_with("Result")))
            .expect("a game point returning Result<(), E> with an API record");
        let error = checked_place("own", &inject(Kind::Replace, target), fallible).unwrap_err();
        assert!(
            error.contains(target) && error.contains(&format!("`{}`", refusal.unwrap())),
            "{error}"
        );
        // An ordinary head listener there is unaffected.
        assert_eq!(
            checked_place("own", &inject(Kind::Head, target), fallible),
            Ok(())
        );
        let &(owned, target, ..) = heads()
            .find(|(.., refusal)| refusal.is_none())
            .expect("a game point a replace can own");
        assert_eq!(
            checked_place("own", &inject(Kind::Replace, target), owned),
            Ok(())
        );
    }
}
