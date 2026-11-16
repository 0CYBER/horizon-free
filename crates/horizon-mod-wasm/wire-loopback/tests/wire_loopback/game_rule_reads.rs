use super::*;
#[test]
fn server_game_rule_reads_loopback() {
    use mod_api::{GameRuleReadError as E, GameRuleValue as V, ServerHost};
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl ServerHost for World {}
    mod_api::impl_server_host_areas!(World, except(config));
    impl mod_api::prelude::ConfigServerHost for World {
        fn game_rule(&self, id: &str) -> Result<V, E> {
            match id {
                "minecraft:keep_inventory" => Ok(V::Bool(true)),
                "minecraft:random_tick_speed" => Ok(V::Int(7)),
                _ => Err(E::UnknownRule),
            }
        }
        fn game_rules(&self) -> Vec<(String, V)> {
            vec![
                ("minecraft:keep_inventory".into(), V::Bool(true)),
                ("minecraft:random_tick_speed".into(), V::Int(7)),
            ]
        }
    }
    let mut frame = boundary::Frame::default();
    frame.set_server_host(&World);
    let mut cx = boundary::Cx::default();
    cx.frames.push(frame);
    let cx = std::cell::RefCell::new(cx);
    guest::set_transport(Some(Rc::new(move |id, bytes| {
        let mut out = Writer::new();
        unsafe { boundary::serve(id, &mut *cx.borrow_mut(), bytes, &mut out) }.unwrap();
        out.into_bytes()
    })));
    let host: &dyn ServerHost = &guest::Guest;
    assert_eq!(
        host.game_rule("minecraft:keep_inventory"),
        Ok(V::Bool(true))
    );
    assert_eq!(host.game_rule("minecraft:random_tick_speed"), Ok(V::Int(7)));
    assert_eq!(host.game_rule("minecraft:not_a_rule"), Err(E::UnknownRule));
    assert_eq!(
        host.game_rules(),
        vec![
            ("minecraft:keep_inventory".into(), V::Bool(true)),
            ("minecraft:random_tick_speed".into(), V::Int(7))
        ]
    );
    guest::set_transport(None);
}
