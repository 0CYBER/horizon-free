//! First-writer attribution for saved-world migration markers.
/// A marked mod id and the first listener that marked it.
#[derive(Clone, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
// exhaustive: marker input and attributed output share this record
pub struct ModMismatchResolution {
    /// The id whose migration was handled, including ids without a difference.
    pub mod_id: String,
    /// The first resolving listener, assigned by dispatch, not trusted from mods.
    pub resolver: String,
}

impl From<String> for ModMismatchResolution {
    fn from(mod_id: String) -> Self {
        Self {
            mod_id,
            resolver: String::new(),
        }
    }
}

pub(crate) fn attribute(
    owner: &str,
    previous: &[ModMismatchResolution],
    resolutions: &mut Vec<ModMismatchResolution>,
) {
    let proposed = std::mem::replace(resolutions, previous.to_vec());
    for marker in proposed {
        if !resolutions.iter().any(|old| old.mod_id == marker.mod_id) {
            resolutions.push(ModMismatchResolution {
                mod_id: marker.mod_id,
                resolver: owner.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_marker_survives_erasure_and_spoofing() {
        let mut markers = vec!["target".to_owned().into()];
        attribute("first", &[], &mut markers);
        assert_eq!(markers[0].resolver, "first");
        let previous = markers.clone();
        markers.clear();
        markers.push(ModMismatchResolution {
            mod_id: "other".into(),
            resolver: "spoof".into(),
        });
        markers.push("target".to_owned().into());
        attribute("second", &previous, &mut markers);
        assert_eq!(
            markers,
            vec![
                ModMismatchResolution {
                    mod_id: "target".into(),
                    resolver: "first".into()
                },
                ModMismatchResolution {
                    mod_id: "other".into(),
                    resolver: "second".into()
                },
            ]
        );
    }
}
