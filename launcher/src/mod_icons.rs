//! Package icons prepared during the background mod-list refresh. Rows keep the image's Arc,
//! so drawing never opens a package or rebuilds an image. Package owns PNG validation.
use horizon_mod_package::icon::Icon;
use std::sync::Arc;

pub(super) struct Loaded {
    pub image: Option<Arc<gpui::Image>>,
    pub error: Option<String>,
}

/// Keeps a refused package distinct from a valid package without an icon.
pub(super) fn load(icon: Result<Option<&Icon>, &String>) -> Loaded {
    match icon {
        Ok(icon) => Loaded {
            image: icon.map(|icon| {
                Arc::new(gpui::Image::from_bytes(
                    gpui::ImageFormat::Png,
                    icon.bytes().to_vec(),
                ))
            }),
            error: None,
        },
        Err(error) => Loaded {
            image: None,
            error: Some(error.clone()),
        },
    }
}
