//! CPU bake proofs: literal positions/UVs, dependency atomicity and fresh resource loads.
use super::*;
use crate::models::StandaloneFormRegistration;
use mod_api::StandaloneModelForm;

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    fn write(&self, name: &str, text: &str) {
        let path = self
            .0
            .path()
            .join(format!("assets/source/models/{name}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn north_arrow_texture(&self) {
        let path = self.0.path().join("assets/source/textures/north_arrow.png");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16usize {
            for x in 0..16usize {
                let shaft = (7..=8).contains(&x) && (5..=14).contains(&y);
                let head = (1..=7).contains(&y) && x.abs_diff(7) < y;
                if shaft || head {
                    pixels[(y * 16 + x) * 4..(y * 16 + x + 1) * 4]
                        .copy_from_slice(&[255, 0, 0, 255]);
                }
            }
        }
        let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }
    fn bake(&self, forms: &[StandaloneFormRegistration]) -> StateModels {
        let source = crate::resource_source::ResourceSource::directory(self.0.path().into());
        let parsed = crate::model_library::ParsedModels::load(&source);
        let mut catalog = StateModels::default();
        catalog.bake_standalone_forms(&source, &mut Sprites::new(source.clone()), &parsed, forms);
        catalog
    }
}
fn registered(form: StandaloneModelForm) -> StandaloneFormRegistration {
    StandaloneFormRegistration {
        owner: "registering_mod",
        form,
    }
}
fn rotated(id: &str, turns: [u8; 3], uvlock: bool) -> StandaloneFormRegistration {
    registered(StandaloneModelForm::Rotated {
        id: id.into(),
        model: "source:arrow".into(),
        turns,
        uvlock,
    })
}
fn close<const N: usize>(actual: [[f32; N]; 4], expected: [[f32; N]; 4]) {
    for (actual, expected) in actual
        .into_iter()
        .flatten()
        .zip(expected.into_iter().flatten())
    {
        assert!(
            (actual - expected).abs() < 0.00001,
            "{actual} != {expected}"
        );
    }
}
fn face(model: &BakedModel, facing: Direction) -> &ModelQuad {
    &model
        .faces
        .iter()
        .find(|face| face.quad.facing == facing)
        .unwrap()
        .quad
}

#[test]
fn standalone_rotated_arrow_has_literal_geometry_and_world_locked_uvs() {
    let fixture = Fixture::new();
    fixture.north_arrow_texture();
    // A north-pointing arrow on the top and an asymmetric UV rectangle on the north wall.
    fixture.write("arrow", r##"{"textures":{"arrow":"source:north_arrow"},"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{"texture":"#arrow","uv":[2,4,10,12]},"up":{"texture":"#arrow","uv":[0,0,16,16]}}}]}"##);
    let models = fixture.bake(&[
        rotated("result:east", [0, 1, 0], false),
        rotated("result:locked", [0, 1, 0], true),
        rotated("result:x", [1, 0, 0], false),
        rotated("result:z", [0, 0, 1], false),
        rotated("result:wrapped", [0, 5, 0], false),
    ]);
    let east = models.standalone("result:east").unwrap();
    let locked = models.standalone("result:locked").unwrap();
    let east_positions = [[1., 1., 1.], [1., 0., 1.], [1., 0., 0.], [1., 1., 0.]];
    close(face(east, Direction::East).positions, east_positions);
    close(face(locked, Direction::East).positions, east_positions);
    // A vertical north wall stays upright under a Y turn, with or without UV locking.
    // The top arrow is where north -> east really rotates texture orientation.
    let wall_uv = [[0.125, 0.25], [0.125, 0.75], [0.625, 0.75], [0.625, 0.25]];
    close(face(east, Direction::East).uv, wall_uv);
    close(face(locked, Direction::East).uv, wall_uv);
    let top_positions = [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]];
    close(face(east, Direction::Up).positions, top_positions);
    close(face(locked, Direction::Up).positions, top_positions);
    close(
        face(east, Direction::Up).uv,
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
    );
    close(
        face(locked, Direction::Up).uv,
        [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
    );
    close(
        face(models.standalone("result:x").unwrap(), Direction::Down).positions,
        [[0., 0., 1.], [0., 0., 0.], [1., 0., 0.], [1., 0., 1.]],
    );
    close(
        face(models.standalone("result:x").unwrap(), Direction::Down).uv,
        [[0.625, 0.75], [0.625, 0.25], [0.125, 0.25], [0.125, 0.75]],
    );
    close(
        face(models.standalone("result:z").unwrap(), Direction::East).positions,
        east_positions,
    );
    close(
        face(models.standalone("result:z").unwrap(), Direction::East).uv,
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
    );
    close(
        face(models.standalone("result:wrapped").unwrap(), Direction::Up).uv,
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
    );
    assert!(
        models.standalone("source:arrow").is_none(),
        "a dependency is not a registration"
    );
}

fn cube(from: [u8; 3], to: [u8; 3], texture: &str) -> String {
    serde_json::json!({"textures":{"all":texture}, "elements":[{
    "from":from, "to":to, "faces":{
        "down":{"texture":"#all"}, "up":{"texture":"#all"},
        "north":{"texture":"#all"}, "south":{"texture":"#all"},
        "west":{"texture":"#all"}, "east":{"texture":"#all"}
    }}]})
    .to_string()
}
fn combined(parts: &[&str]) -> StandaloneFormRegistration {
    registered(StandaloneModelForm::Combined {
        id: "different_namespace:assembly".into(),
        parts: parts.iter().map(|part| (*part).into()).collect(),
    })
}

#[test]
fn standalone_combined_keeps_both_cubes_and_rebakes_changed_dependencies() {
    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    fixture.write("overlay", &cube([8, 8, 8], [16, 16, 16], "source:overlay"));
    let forms = [combined(&["source:base", "source:overlay"])];
    let first = fixture.bake(&forms);
    let model = first.standalone("different_namespace:assembly").unwrap();
    assert_eq!(model.faces.len(), 12);
    close(
        face(model, Direction::North).positions,
        [[0.5, 0.5, 0.], [0.5, 0., 0.], [0., 0., 0.], [0., 0.5, 0.]],
    );
    let overlay = model.faces[6..]
        .iter()
        .find(|f| f.quad.facing == Direction::North)
        .unwrap();
    close(
        overlay.quad.positions,
        [
            [1., 1., 0.5],
            [1., 0.5, 0.5],
            [0.5, 0.5, 0.5],
            [0.5, 1., 0.5],
        ],
    );
    assert_eq!(
        model
            .faces
            .iter()
            .filter(|f| f.quad.material.sprite == "source:base")
            .count(),
        6
    );
    assert_eq!(
        model
            .faces
            .iter()
            .filter(|f| f.quad.material.sprite == "source:overlay")
            .count(),
        6
    );
    assert!(first.standalone("source:base").is_none());
    assert!(first.standalone("source:overlay").is_none());
    fixture.write("overlay", &cube([12, 0, 0], [16, 4, 4], "source:changed"));
    let second = fixture.bake(&forms);
    let model = second.standalone("different_namespace:assembly").unwrap();
    assert_eq!(model.faces.len(), 12);
    let overlay = model.faces[6..]
        .iter()
        .find(|f| f.quad.facing == Direction::North)
        .unwrap();
    close(
        overlay.quad.positions,
        [
            [1., 0.25, 0.],
            [1., 0., 0.],
            [0.75, 0., 0.],
            [0.75, 0.25, 0.],
        ],
    );
    assert_eq!(overlay.quad.material.sprite, "source:changed");
}

#[test]
fn standalone_missing_dependency_drops_entire_form_and_logs_registering_mod() {
    let fixture = Fixture::new();
    fixture.write("base", &cube([0, 0, 0], [8, 8, 8], "source:base"));
    fixture.write("orphan", r#"{"parent":"source:missing_parent"}"#);
    let (models, logs) = crate::test_log::capture(|| {
        fixture.bake(&[
            combined(&["source:base", "source:orphan"]),
            rotated("third_namespace:missing_rotated", [0, 1, 0], true),
        ])
    });
    assert!(models.standalone("different_namespace:assembly").is_none());
    assert!(
        models
            .standalone("third_namespace:missing_rotated")
            .is_none()
    );
    assert!(
        models.baked.is_empty(),
        "no partially baked dependency is published"
    );
    {
        for id in [
            "different_namespace:assembly",
            "third_namespace:missing_rotated",
        ] {
            assert!(
                logs.iter()
                    .any(|line| line.contains("mod registering_mod standalone form")
                        && line.contains(id)),
                "{logs:?}"
            );
        }
    }
}
