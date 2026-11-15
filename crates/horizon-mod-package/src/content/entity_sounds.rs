//! Declarative sound selection; omission remains distinct from explicit silence.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntitySounds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<SoundOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hurt: Option<SoundOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death: Option<SoundOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<SoundOverride>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum SoundOverride {
    Mute(Mute),
    Id(String),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum Mute {
    #[serde(rename = "none")]
    None,
}
impl EntitySounds {
    pub fn validate(&self, entity: &str) -> Result<(), String> {
        for (slot, value) in [
            ("ambient", &self.ambient),
            ("hurt", &self.hurt),
            ("death", &self.death),
            ("step", &self.step),
        ] {
            if let Some(SoundOverride::Id(id)) = value {
                let valid = id.split_once(':').is_some_and(|(n, p)| {
                    !n.is_empty()
                        && !p.is_empty()
                        && n.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b)
                        })
                        && p.bytes().all(|b| {
                            b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-/".contains(&b)
                        })
                });
                if !valid {
                    return Err(format!(
                        "{entity}: sounds.{slot}: invalid full sound id {id:?}"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "entity_sounds_tests.rs"]
mod tests;
