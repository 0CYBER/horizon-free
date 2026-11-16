/// The in-memory F2 frame (`ScreenshotEvent.getImage`), before either file is written.
///
/// RGBA8, top to bottom: `rgba.len()` is `width * height * 4`. Change the pixels, or
/// replace the whole image, and the normal PNG is what remains. A length other than
/// `width * height * 4` writes nothing and reports screenshot failure. The HDR copy
/// is not this image.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ScreenshotImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA8.
    pub rgba: Vec<u8>,
}

impl ScreenshotImage {
    /// An RGBA8 frame of `width` by `height`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba,
        }
    }
}
