#[cfg(test)]
mod image_parameter_tests {
    use crate::{Macros, Pack, preprocess, translate_compute};
    #[test]
    fn nested_image_helpers_specialize_multiple_storage_formats() {
        let src = "#version 430\nlayout(local_size_x=1)in;layout(rgba16f) uniform image3D imageA;layout(r32f) uniform image3D imageB;void write_value(writeonly image3D img,ivec3 p){imageStore(img,p,vec4(0.5));}\nvoid nested(image3D img,ivec3 p){write_value(img,p);}\nvoid main(){nested(imageA,ivec3(0));nested(imageB,ivec3(0));}";
        let pack = Pack::from_zip_bytes(
            "formats.zip",
            crate::pack::tests::zip(&[("shaders/setup.csh", src)]),
        )
        .unwrap();
        let result = translate_compute(&preprocess(&pack, "setup.csh", &Macros::standard()));
        assert!(result.is_ok(), "{:?}", result.err());
    }
}

#[cfg(test)]
mod mixed_integer_bounds_tests {
    #[test]
    fn compute_clamp_with_signed_index_and_unsigned_bounds_compiles() {
        let source = "#version 430\nlayout(local_size_x=1) in; layout(std430,binding=0) buffer Data{uint result;};void main(){int index=int(gl_LocalInvocationID.x);result=clamp(index,0u,47u);}";
        let pack = crate::Pack::from_zip_bytes(
            "integer-bounds.zip",
            crate::pack::tests::zip(&[("shaders/setup.csh", source)]),
        )
        .unwrap();
        let stage = crate::preprocess(&pack, "setup.csh", &crate::Macros::standard());
        let result = crate::translate_compute(&stage);
        assert!(result.is_ok(), "{:?}", result.err());
    }
}
