#[test]
fn posed_parts_are_the_poses_the_draw_made_and_change_nothing() {
    use super::super::anim;
    let (mob, data) = humanoid_zombie();
    let atlas = |_: &str| Some([0.0, 0.0, 1.0, 1.0]);
    let draw = |mob: &Mob, keep: bool| {
        let (mut out, mut parts) = (Vec::new(), None);
        render_with_hidden(
            mob,
            0.5,
            glam::DVec3::ZERO,
            &atlas,
            0xf0f0,
            &data,
            &Default::default(),
            &[],
            Default::default(),
            keep.then_some(&mut parts),
            &mut out,
        );
        (
            out.iter().map(|v| format!("{v:?}")).collect::<Vec<_>>(),
            parts,
        )
    };
    let (before, none) = draw(&mob, false);
    assert!(none.is_none(), "nothing kept unless asked");
    let (after, parts) = draw(&mob, true);
    assert_eq!(after, before, "keeping the poses changes nothing drawn");
    let parts = parts.expect("a head and a body: a humanoid");

    // Independently: the model's poses after the zombie's setupAnim for this frame.
    let mut poses = anim::Poses::new(&mob.kind.model);
    anim::setup(&mob, &anim::state(&mob, 0.5), &mut poses, &data.animations);
    for part in mod_api::HumanoidPart::ALL {
        let expected = poses.pose_of(part.model_name());
        let got = parts.get(part).map(|p| {
            let q = p.pose;
            [
                q.x, q.y, q.z, q.x_rot, q.y_rot, q.z_rot, q.x_scale, q.y_scale, q.z_scale,
            ]
        });
        assert_eq!(got, expected, "{part:?}");
    }
    let arm = parts.get(mod_api::HumanoidPart::RightArm).unwrap();
    assert!(
        arm.pose.x_rot != 0.0 && arm.visible,
        "a zombie holds its arms out"
    );
    assert!(
        parts.get(mod_api::HumanoidPart::RightSleeve).is_none(),
        "a zombie has no sleeves"
    );

    // The GPU path and the layers-only draw (an invisible player's armour) keep the same.
    let mut models = super::super::skin::SkinModels::default();
    let mut frame = crate::gfx::entity_skin::SkinFrame::default();
    let mut skin = super::super::skin::Skin::new(&mut models, &mut frame);
    let mut skinned = None;
    render_skinned(
        &mob,
        0.5,
        glam::DVec3::ZERO,
        &atlas,
        0xf0f0,
        &data,
        &Default::default(),
        Default::default(),
        Some(&mut skinned),
        &mut Vec::new(),
        &mut skin,
    );
    assert_eq!(skinned, Some(parts), "the GPU path");
    let mut layered = None;
    render_layers(
        &mob,
        0.5,
        glam::DVec3::ZERO,
        &atlas,
        0xf0f0,
        &data,
        &Default::default(),
        &[],
        Some(&mut layered),
        &mut Vec::new(),
    );
    assert_eq!(layered, Some(parts), "the layers over an unseen body");

    // QuadrupedModel (a cow) has a head and a body too: its family, not its parts, says it
    // isn't humanoid.
    let cube = |x: f32| serde_json::json!({ "pose": [x, 0.0, 0.0], "polygons": [[[x, 0.0, 0.0, 0.0, 0.0], [x + 1.0, 0.0, 0.0, 1.0, 0.0], [x + 1.0, 1.0, 0.0, 1.0, 1.0], [x, 1.0, 0.0, 0.0, 1.0]]] });
    let quadruped = serde_json::json!({ "children": {
        "head": cube(0.0), "body": cube(0.0), "right_hind_leg": cube(-2.0),
        "left_hind_leg": cube(2.0), "right_front_leg": cube(-2.0), "left_front_leg": cube(2.0),
    }});
    let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
    kind.species = Species::Cow;
    kind.anim = super::super::Anim::Quadruped;
    kind.model = std::sync::Arc::new(Part::parse("root", &quadruped));
    let mut cow = Mob::new(
        2,
        std::sync::Arc::new(kind),
        horizon_core::geometry::Vec3::ZERO,
        0.0,
    );
    cow.age = 37;
    let (cow_drawn, cow_parts) = draw(&cow, true);
    assert!(!cow_drawn.is_empty(), "the cow draws");
    assert!(cow_parts.is_none(), "a cow is no humanoid");

    // `render_living_pre` hides a part of any model by its name, on the CPU and on the GPU
    // (arismoko/horizon#2957): a leg's one face goes; an unknown name is refused.
    let mut leg = mod_api::HiddenParts::default();
    leg.hide_named("right_hind_leg").unwrap();
    let mut absent = mod_api::HiddenParts::default();
    absent.hide_named("nose").unwrap();
    let cpu = |mob: &Mob, hidden| {
        let mut out = Vec::new();
        render_with_hidden(
            mob,
            0.5,
            glam::DVec3::ZERO,
            &atlas,
            0xf0f0,
            &data,
            &Default::default(),
            &[],
            hidden,
            None,
            &mut out,
        );
        out.len()
    };
    let whole = cpu(&cow, Default::default());
    assert_eq!(
        cpu(&cow, leg),
        whole - 4,
        "the leg's face leaves the CPU draw"
    );
    let mut poses = Poses::pooled();
    let name = hide_parts(&mut poses, &absent).unwrap_err();
    assert_eq!(name, "nose");
    assert_eq!(cpu(&cow, absent), 0, "unknown names refuse the draw");
    let gpu = |mob: &Mob, hidden| {
        let mut models = super::super::skin::SkinModels::default();
        let mut frame = crate::gfx::entity_skin::SkinFrame::default();
        let mut skin = super::super::skin::Skin::new(&mut models, &mut frame);
        render_skinned(
            mob,
            0.5,
            glam::DVec3::ZERO,
            &atlas,
            0xf0f0,
            &data,
            &Default::default(),
            hidden,
            None,
            &mut Vec::new(),
            &mut skin,
        );
        frame.jobs.iter().map(|job| job.count).sum::<u32>()
    };
    let whole = gpu(&cow, Default::default());
    assert!(whole > 0, "the cow takes the GPU path");
    assert_eq!(
        gpu(&cow, leg),
        whole - 4,
        "the leg's face leaves the GPU draw"
    );
    // Villager hat_rim is a child of hat; overlays and armour retain the full tree.
    let mut hat = cube(0.0);
    hat["children"] = serde_json::json!({"hat_rim": cube(0.0)});
    let mut head = cube(0.0);
    head["children"] = serde_json::json!({"hat": hat, "nose": cube(0.0)});
    let tree = serde_json::json!({"children": {"head": head, "body": cube(0.0)}});
    let mut kind = (*cow.kind).clone();
    kind.id = "minecraft:villager".into();
    kind.species = Species::Villager;
    kind.anim = super::super::Anim::Villager;
    kind.model = std::sync::Arc::new(Part::parse("root", &tree));
    let villager = Mob::new(
        3,
        std::sync::Arc::new(kind),
        horizon_core::geometry::Vec3::ZERO,
        0.0,
    );
    let mut hidden = mod_api::HiddenParts::default();
    hidden.hide_named("hat_rim").unwrap();
    hidden.hide_named("nose").unwrap();
    assert_eq!(
        cpu(&villager, hidden),
        cpu(&villager, Default::default()) - 8
    );
    assert_eq!(
        gpu(&villager, hidden),
        gpu(&villager, Default::default()) - 8
    );
    let armour = [(
        &*villager.kind.model,
        [0.0, 0.0, 1.0, 1.0],
        0xf0f0,
        0,
        false,
    )];
    for gpu_path in [false, true] {
        let draw_armoured = |hidden, extra: &[(&Part, [f32; 4], u32, u32, bool)]| {
            let mut out = Vec::new();
            let mut models = super::super::skin::SkinModels::default();
            let mut frame = crate::gfx::entity_skin::SkinFrame::default();
            let mut skin = super::super::skin::Skin::new(&mut models, &mut frame);
            render_visible(
                &villager,
                0.5,
                glam::Mat4::IDENTITY,
                &atlas,
                0xf0f0,
                &data,
                &Default::default(),
                extra,
                Some(body_color(&villager, 0.5)),
                hidden,
                None,
                &mut out,
                gpu_path.then_some(&mut skin),
            );
            out.len() as u32 + frame.jobs.iter().map(|job| job.count).sum::<u32>()
        };
        let bare = draw_armoured(Default::default(), &[]);
        let armoured = draw_armoured(Default::default(), &armour);
        assert!(armoured > bare);
        assert_eq!(
            draw_armoured(hidden, &armour),
            armoured - 8,
            "armour retains every part on CPU and GPU"
        );
    }
    let mut parent = mod_api::HiddenParts::default();
    parent.hide_named("hat").unwrap();
    assert_eq!(
        cpu(&villager, parent),
        cpu(&villager, Default::default()) - 8
    );
    assert_eq!(
        gpu(&villager, parent),
        gpu(&villager, Default::default()) - 8
    );
    // VillagerProfessionLayer draws the type, profession and level on `getParentModel()`:
    // a part hidden on the body leaves them too.
    let mut kind = (*villager.kind).clone();
    kind.layers
        .insert("villager#main".into(), kind.model.clone());
    let mut layered = Mob::new(
        4,
        std::sync::Arc::new(kind),
        horizon_core::geometry::Vec3::ZERO,
        0.0,
    );
    let table = super::super::villager::ResolvedProfessions::vanilla();
    super::villager_client::install(&table, &Default::default());
    layered.extra.profession = super::super::villager::ProfessionState::Vanilla(5);
    assert_eq!(
        layers(&layered, 0.0, &Default::default()).len(),
        3,
        "type, profession and level"
    );
    let mut nose = mod_api::HiddenParts::default();
    nose.hide_named("nose").unwrap();
    assert_eq!(
        cpu(&layered, nose),
        cpu(&layered, Default::default()) - 16,
        "the nose leaves the body and every layer on it"
    );
    assert_eq!(gpu(&layered, nose), gpu(&layered, Default::default()) - 16,);
    // A full profession hat makes the type overlay use the no-hat poses.
    // Its head (including nose) is absent even before the mod hides the nose;
    // the body, profession and level each lose one four-vertex polygon.
    use horizon_assets::resource_source::{Resource, ResourceSource};
    let source = ResourceSource::indexed(std::collections::BTreeMap::from([
        (
            "assets/minecraft/textures/entity/villager/profession/farmer.png".into(),
            Resource::bytes(vec![]),
        ),
        (
            "assets/minecraft/textures/entity/villager/profession/farmer.png.mcmeta".into(),
            Resource::bytes(br#"{"villager":{"hat":"full"}}"#.to_vec()),
        ),
    ]));
    super::villager_client::install(
        &table,
        &super::super::villager_hats::VillagerHats::load(&source),
    );
    assert!(!super::villager_client::type_hat_shown(
        &layered.extra.profession,
        false,
        layered.extra.variant
    ));
    assert_eq!(cpu(&layered, nose), cpu(&layered, Default::default()) - 12);
    assert_eq!(gpu(&layered, nose), gpu(&layered, Default::default()) - 12);
    super::villager_client::install(&table, &Default::default());
}
