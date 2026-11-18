//! Original contract control from accepted shader-format spec 3.2.
use horizon_shaders::{Macros, Properties};

#[test]
fn required_per_buffer_blending_is_recognized_without_foreign_macros() {
    let mut macros = Macros::standard();
    macros.define_renderer(4);
    let properties = Properties::parse("iris.features.required = per_buffer_blending\n", &macros);
    assert!(Macros::feature_notes(&properties).is_empty());
    assert!(!macros.is_defined("IRIS_FEATURE_PER_BUFFER_BLENDING"));
    assert!(!macros.is_defined("IS_IRIS"));
}
