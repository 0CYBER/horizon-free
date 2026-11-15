//! A package's optional, bounded PNG icon.

/// Maximum encoded size, also used for registry icons.
pub const MAX_ICON: usize = 256 << 10;
const MAX_DIMENSION: u32 = 512;

/// Encoded PNG bytes validated at the package boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon(Vec<u8>);

impl Icon {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_ICON {
            return Err("icon.png exceeds 256 KiB".into());
        }
        let mut decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
        decoder.set_limits(png::Limits { bytes: 4 << 20 });
        let mut reader = decoder.read_info().map_err(|e| format!("icon.png: {e}"))?;
        let (width, height) = (reader.info().width, reader.info().height);
        if width == 0 || width != height || width > MAX_DIMENSION {
            return Err("icon.png must be square and at most 512×512".into());
        }
        let mut pixels = vec![0; reader.output_buffer_size().ok_or("icon.png is too large")?];
        let frames = reader
            .info()
            .animation_control
            .as_ref()
            .map_or(1, |animation| {
                u64::from(animation.num_frames) + u64::from(reader.info().frame_control.is_none())
            });
        for _ in 0..frames {
            reader
                .next_frame(&mut pixels)
                .map_err(|e| format!("icon.png: {e}"))?;
        }
        reader.finish().map_err(|e| format!("icon.png: {e}"))?;
        Ok(Self(bytes))
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer
                .write_image_data(&vec![0; (width * height * 4) as usize])
                .unwrap();
        }
        bytes
    }

    #[test]
    fn package_reads_icons_from_folder_and_archive() {
        use crate::manifest::{MANIFEST, Package};
        let manifest = "id = 'example'\nname = 'Example'\nversion = '1'\napi = '0.2'\ncapabilities = ['registries']\n";
        let root = std::env::temp_dir().join(format!("horizon-icon-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        assert!(Package::read(&root).unwrap().icon.is_none());
        let bytes = png(4, 4);
        std::fs::write(root.join("icon.png"), &bytes).unwrap();
        let archive = root.join("example.hmod");
        std::fs::write(
            &archive,
            Package::write_files(manifest, &[("icon.png", &bytes)]).unwrap(),
        )
        .unwrap();
        for path in [&root, &archive] {
            assert_eq!(Package::read(path).unwrap().icon.unwrap().bytes(), bytes);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_invalid_icons() {
        let valid = png(4, 4);
        assert_eq!(Icon::parse(valid.clone()).unwrap().bytes(), valid);
        assert!(Icon::parse(png(4, 3)).is_err());
        assert!(Icon::parse(png(513, 513)).is_err());
        assert!(Icon::parse(vec![0; MAX_ICON + 1]).is_err());
        assert!(Icon::parse(b"not a png".to_vec()).is_err());
        assert!(Icon::parse(valid[..valid.len() / 2].to_vec()).is_err());
    }
}
