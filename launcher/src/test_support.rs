/// Fail before capability-dependent fixtures silently choose the legacy launch path.
pub(crate) fn assert_holder_capability() {
    horizon_platform::mod_package::holder::retain_capability();
    assert!(
        horizon_platform::mod_package::holder::capable(&std::env::current_exe().unwrap()).unwrap(),
        "capability marker missing from this binary (release optimisation?)"
    );
}

#[test]
fn current_binary_retains_holder_capability() {
    assert_holder_capability();
}

/// A `w` × `h` PNG of one colour, as a file holds it.
pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(w, h, image::Rgba([40, 160, 90, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

/// The size of the picture in `bytes`.
pub(crate) fn dimensions(bytes: &[u8]) -> (u32, u32) {
    let image = image::load_from_memory(bytes).unwrap();
    (image.width(), image.height())
}
