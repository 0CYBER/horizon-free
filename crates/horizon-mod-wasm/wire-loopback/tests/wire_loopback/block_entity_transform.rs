use super::*;
struct Step;
impl mod_api::BlockEntityTransform for Step {
    fn transform(
        &self,
        data: &mod_api::Compound,
        transform: mod_api::StructureTransform,
    ) -> Result<mod_api::Compound, mod_api::TransformRefusal> {
        if data.contains_key(&"refuse".into()) {
            return Err(mod_api::TransformRefusal::new("wire fixture refusal"));
        }
        let facing = match (transform.rotation, transform.mirror) {
            (mod_api::StructureRotation::None, mod_api::StructureMirror::None) => "north",
            (mod_api::StructureRotation::Clockwise90, mod_api::StructureMirror::None) => "east",
            (mod_api::StructureRotation::Clockwise180, mod_api::StructureMirror::None) => "south",
            (mod_api::StructureRotation::Counterclockwise90, mod_api::StructureMirror::None) => {
                "west"
            }
            (mod_api::StructureRotation::None, mod_api::StructureMirror::LeftRight) => "south",
            (mod_api::StructureRotation::None, mod_api::StructureMirror::FrontBack) => "north",
            _ => return Err(mod_api::TransformRefusal::new("unexpected transform")),
        };
        let mut result = data.clone();
        result.insert("facing".into(), mod_api::Tag::String(facing.into()));
        Ok(result)
    }
}
pub(super) fn register(r: &mut dyn mod_api::Registrar) {
    r.block_entity_type("turning").transform(Step).build();
}
#[test]
fn declaration_transform_round_trips_through_generated_guest_and_host() {
    install_counter();
    let step = mod_api::__private::block_entity_transform("wire_test:turning").unwrap();
    let data = [("facing".into(), mod_api::Tag::String("north".into()))].into();
    for (rotation, mirror, expected) in [
        (
            mod_api::StructureRotation::None,
            mod_api::StructureMirror::None,
            "north",
        ),
        (
            mod_api::StructureRotation::Clockwise90,
            mod_api::StructureMirror::None,
            "east",
        ),
        (
            mod_api::StructureRotation::Clockwise180,
            mod_api::StructureMirror::None,
            "south",
        ),
        (
            mod_api::StructureRotation::Counterclockwise90,
            mod_api::StructureMirror::None,
            "west",
        ),
        (
            mod_api::StructureRotation::None,
            mod_api::StructureMirror::LeftRight,
            "south",
        ),
        (
            mod_api::StructureRotation::None,
            mod_api::StructureMirror::FrontBack,
            "north",
        ),
    ] {
        let output = step
            .transform(&data, mod_api::StructureTransform::new(rotation, mirror))
            .unwrap();
        assert_eq!(
            output.get(&"facing".into()),
            Some(&mod_api::Tag::String(expected.into()))
        );
    }
    let refused = [("refuse".into(), mod_api::Tag::Byte(1))].into();
    assert_eq!(
        step.transform(
            &refused,
            mod_api::StructureTransform::new(
                mod_api::StructureRotation::None,
                mod_api::StructureMirror::None
            )
        )
        .unwrap_err()
        .reason,
        "wire fixture refusal"
    );
}

pub(super) struct BlockEntityTransformHooks;
impl BlockEntityTransformHooks {
    pub(super) fn register(
        &self,
        registrar: &mut dyn mod_api::Registrar,
        cx: &mod_api::RegistrationContext,
    ) {
        self::register(registrar);
    }
}
