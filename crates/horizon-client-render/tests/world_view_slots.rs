use glam::{DVec3, Mat4};
use horizon_client_render::render_slots::{
    self, Slots, WorldView, WorldViewDrawn, WorldViewRefusal, WorldViewRefusalReason, WorldViews,
};
use std::sync::Mutex;
static DELIVERED: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static REFUSED: Mutex<Vec<Result<(), (u64, WorldViewRefusalReason)>>> = Mutex::new(Vec::new());

#[test]
#[ignore = "requires a Vulkan/Metal/DX12 adapter"]
fn world_view_slot_delivers_only_drawable_requests() {
    assert!(!render_slots::world_drawn_hooked());
    assert!(!render_slots::needs().entity_spans);
    assert!(!render_slots::needs().world_depth);
    render_slots::install(Slots {
        world_views: |views| {
            let named = |r: WorldViewRefusal| (r.id, r.reason);
            let mut pushed = REFUSED.lock().unwrap();
            for (id, size) in [(41, [4, 4]), (42, [0, 4])] {
                let view = WorldView::new(id, DVec3::ZERO, 0.0, 0.0, 70.0, size);
                pushed.push(views.push(view).map_err(named));
            }
        },
        world_views_hooked: || true,
        world_view_drawn: |view| {
            DELIVERED.lock().unwrap().push(view.id);
        },
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
    // Real draw data reaches the installed subscriber; the excluded request never does.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = color.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(render_slots::world_views_hooked());
    let mut asked = WorldViews::new(4096);
    render_slots::world_views(&mut asked);
    assert_eq!(
        *REFUSED.lock().unwrap(),
        [Ok(()), Err((42, WorldViewRefusalReason::Side([0, 4])))]
    );
    for view in asked.views() {
        render_slots::world_view_drawn(&mut WorldViewDrawn {
            id: view.id,
            parent: view.parent,
            nesting_depth: 1,
            device: &device,
            queue: &queue,
            encoder: &mut encoder,
            color: &color,
            color_view: &target,
            color_format: wgpu::TextureFormat::Rgba8Unorm,
            size: view.size,
            depth: &target,
            view_proj: Mat4::IDENTITY,
            camera: DVec3::ZERO,
        });
    }
    assert_eq!(*DELIVERED.lock().unwrap(), [41]);
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
