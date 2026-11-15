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
    let listed: Vec<(u32, String)> = module
        .call(
            CallClass::Load,
            Frame::default(),
            0xBFFF_FFFF,
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
    for (index, (place, id)) in listed.into_iter().enumerate() {
        let inject = declared
            .remove(id.as_str())
            .ok_or_else(|| format!("undeclared point listener {id}"))?;
        let point =
            checked_place(&id, inject, place).map_err(|e| format!("{}: {e}", manifest.id))?;
        slots.push((
            point,
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
    for (place, listener, priority, id, replace) in slots {
        let endpoint = Box::leak(Box::new(Endpoint {
            owner,
            module: module.downgrade(),
            listener,
        }));
        let id = Box::leak(id.into_boxed_str());
        let mut slot =
            SlotListener::forwarded(owner, place.place(), endpoint, 0x8000_0000 + place as u32)
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

/// The engine's API place `place` that a guest subscribed as its declared `inject`, checked
/// against the game's own point list, not what the module says of itself: the target, the phase,
/// and, for a `replace`, that its head returns `()` or an API value the listener can produce.
fn checked_place(id: &str, inject: &Inject, place: u32) -> Result<PointId, String> {
    let Some(&(point, target, at_return, replace_refusal)) = mod_points::API_PLACES
        .iter()
        .find(|(point, ..)| *point as u32 == place)
    else {
        return Err(format!(
            "{id}: {}: no API record for place {place}",
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
    Ok(point)
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
            (func (export "horizon_hook") (param i32) (result i64) i64.const 0))"#,
                *mod_api::wire::LAYOUT
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
            (func (export "horizon_hook") (param i32) (result i64) i64.const 0))"#,
                *mod_api::wire::LAYOUT
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

    #[test]
    fn missing_place_names_mod_row_and_declared_target() {
        let error =
            checked_place("addon:own", &inject(Kind::Head, "game::tick"), u32::MAX).unwrap_err();
        assert_eq!(
            error,
            "addon:own: game::tick: no API record for place 4294967295"
        );
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
        let error =
            checked_place("own", &inject(Kind::Replace, target), fallible as u32).unwrap_err();
        assert!(
            error.contains(target) && error.contains(&format!("`{}`", refusal.unwrap())),
            "{error}"
        );
        // An ordinary head listener there is unaffected.
        assert_eq!(
            checked_place("own", &inject(Kind::Head, target), fallible as u32),
            Ok(fallible)
        );
        let &(owned, target, ..) = heads()
            .find(|(.., refusal)| refusal.is_none())
            .expect("a game point a replace can own");
        assert_eq!(
            checked_place("own", &inject(Kind::Replace, target), owned as u32),
            Ok(owned)
        );
    }
}
