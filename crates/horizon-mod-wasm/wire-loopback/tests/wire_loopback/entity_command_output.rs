use super::*;
use mod_api::__private::game::BackendMutRefOf;

struct Server;
impl mod_api::Host for Server {}
mod_api::impl_host_areas!(Server);
impl mod_api::ServerHost for Server {
    fn backend_mut(&mut self) -> Option<mod_api::BackendMutRef<'_>> {
        Some(mod_api::BackendMutRef::of(self))
    }
}
mod_api::impl_server_host_areas!(Server);
impl mod_api::Backend for Server {}
mod_api::impl_backend_areas!(Server);
impl mod_api::BackendMut for Server {}
mod_api::impl_backend_mut_areas!(Server, except(entities));
impl mod_api::prelude::EntitiesBackendMut for Server {
    fn entity_run_command_with_output(
        &mut self,
        entity: mod_api::EntityRef,
        line: &str,
        level: mod_api::PermissionLevel,
    ) -> mod_api::CommandResult {
        assert_eq!(
            entity,
            mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 5)
        );
        assert_eq!(level.level(), 2);
        match line {
            "/success" => mod_api::CommandResult::new(true, vec!["first".into(), "second".into()]),
            "/failure" => {
                mod_api::CommandResult::new(false, vec!["failed".into(), "detail".into()])
            }
            "/empty" => mod_api::CommandResult::new(true, vec![]),
            _ => panic!("unexpected wire command"),
        }
    }
}
struct NoBackend;
impl mod_api::Host for NoBackend {}
mod_api::impl_host_areas!(NoBackend);
impl mod_api::ServerHost for NoBackend {}
mod_api::impl_server_host_areas!(NoBackend);

#[test]
fn command_output_proxy_preserves_inputs_order_and_refusal() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    let mut server = Server;
    let requests = Rc::new(std::cell::RefCell::new(Vec::new()));
    {
        let host = &mut server;
        let mut frame = boundary::Frame::default();
        frame.set_server_host_mut(host);
        let mut cx = Box::new(boundary::Cx::default());
        cx.frames.push(frame);
        let pointer = &mut *cx as *mut boundary::Cx;
        // SAFETY: the context and borrowed host outlive synchronous proxy calls.
        let transport = unsafe { host_dispatch::transport(pointer) };
        let seen = requests.clone();
        let transport: guest::Transport = Rc::new(move |id, bytes| {
            seen.borrow_mut().push((id, bytes.to_vec()));
            transport(id, bytes)
        });
        let previous =
            LOOPBACK_TRANSPORT.with(|active| active.borrow_mut().replace(transport.clone()));
        let _restore = TransportScope(previous);
        guest::set_transport(Some(transport));
        let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 5);
        for (line, success, feedback) in [
            ("/success", true, vec!["first", "second"]),
            ("/failure", false, vec!["failed", "detail"]),
            ("/empty", true, vec![]),
        ] {
            let result = entity.run_command_with_output(
                &mut guest::Guest,
                line,
                mod_api::PermissionLevel::from_level(2),
            );
            assert_eq!(result.success, success);
            assert_eq!(result.feedback, feedback);
        }
    }
    // The native handle's documented no-backend value is not a wire service reply:
    // the service refuses a missing backend before it can invoke the method.
    let mut absent = NoBackend;
    let entity = mod_api::EntityRef::new(mod_api::WorldEpoch::new(7), 5);
    for (line, _, _) in [
        ("/success", true, vec!["first", "second"]),
        ("/failure", false, vec!["failed", "detail"]),
        ("/empty", true, vec![]),
    ] {
        let result = entity.run_command_with_output(
            &mut absent,
            line,
            mod_api::PermissionLevel::from_level(2),
        );
        assert!(!result.success);
        assert!(result.feedback.is_empty());
    }
    let mut frame = boundary::Frame::default();
    frame.set_server_host_mut(&mut absent);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    assert_eq!(requests.borrow().len(), 3);
    for (id, bytes) in requests.borrow().iter() {
        let mut out = Writer::new();
        // SAFETY: the context and borrowed host remain live for this synchronous call.
        let result = unsafe { boundary::serve(*id, &mut cx, bytes, &mut out) };
        assert!(matches!(
            result,
            Err(boundary::ServeError::Missing("game backend"))
        ));
    }
}
