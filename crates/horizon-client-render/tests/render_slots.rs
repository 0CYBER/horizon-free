use horizon_client_render::render_slots::{self, Slots};

#[test]
fn render_provider_is_optional_and_installed_once() {
    assert!(!render_slots::world_drawn_hooked());
    assert!(!render_slots::needs().entity_spans);
    assert!(!render_slots::needs().world_depth);
    render_slots::install(Slots {
        world_views: |_| {},
        world_views_hooked: || false,
        world_view_drawn: |_| {},
        world_drawn: |_| {},
        hand_drawn: |_| {},
        render_needs: |needs| {
            needs.entity_spans = true;
            needs.world_depth = true;
        },
        world_drawn_hooked: || true,
    });
    assert!(render_slots::world_drawn_hooked());
    assert!(render_slots::needs().entity_spans);
    assert!(render_slots::needs().world_depth);
    render_slots::install(Slots {
        world_views: |_| {},
        world_views_hooked: || false,
        world_view_drawn: |_| {},
        world_drawn: |_| panic!("second provider replaced the first"),
        hand_drawn: |_| panic!("second provider replaced the first"),
        render_needs: |_| panic!("second provider replaced the first"),
        world_drawn_hooked: || false,
    });
    assert!(render_slots::world_drawn_hooked());
    assert!(render_slots::needs().world_depth);
}
