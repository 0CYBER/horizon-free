use super::*;
use mod_api::{ClientHost, DrawContext, Flow, TabListLatencyDrawEvent, TabListLatencyWidthEvent};

const UUID: u128 = 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210;
const BOUNDS: [i32; 4] = [180, 28, 234, 36];

pub(super) struct TabLatencyHooks;
impl TabLatencyHooks {
    pub(super) fn tab_list_latency_width(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::TabListLatencyWidthEvent,
    ) {
        assert_eq!(event.width, 13);
        event.width = 54;
    }

    pub(super) fn tab_list_latency_draw(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::TabListLatencyDrawEvent<'_>,
    ) -> mod_api::Flow {
        assert_eq!(event.player_uuid, UUID);
        assert_eq!(event.player_name, "Rain");
        assert_eq!(event.latency_ms, -1);
        assert_eq!(event.bounds, BOUNDS);
        event.draw.fill([181.0, 29.0, 10.0, 6.0], 0xff50_7060);
        Flow::Cancel
    }
}

#[test]
fn tab_list_latency_width_and_clipped_draw_contract_cross_wire() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    struct Client;
    impl mod_api::Host for Client {}
    mod_api::impl_host_areas!(Client);
    impl ClientHost for Client {}
    mod_api::impl_client_host_areas!(Client);

    #[derive(Default)]
    struct Draw(Option<([f32; 4], u32)>);
    impl DrawContext for Draw {
        fn fill(&mut self, rect: [f32; 4], argb: u32) {
            self.0 = Some((rect, argb));
        }
    }

    let guest = WasmMod {
        group: 0,
        subscribed: &[],
        listeners: &[],
        corrupt: false,
    };
    let mut width = TabListLatencyWidthEvent { width: 13 };
    guest.tab_list_latency_width(&Client, &mut width);
    assert_eq!(width.width, 54);
    let mut draw = Draw::default();
    let mut event = TabListLatencyDrawEvent {
        player_uuid: UUID,
        player_name: "Rain",
        latency_ms: -1,
        bounds: BOUNDS,
        draw: &mut draw,
        cancelled: false,
    };
    assert_eq!(guest.tab_list_latency_draw(&Client, &mut event), Flow::Cancel);
    assert_eq!(draw.0, Some(([181.0, 29.0, 10.0, 6.0], 0xff50_7060)));
}
