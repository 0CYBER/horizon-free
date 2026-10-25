//! Display/TextDisplay synced data (26.3), checked against vanilla accessors
//! and ClientboundSetEntityDataPacket's own stream codec.
use crate::text::Text;
use horizon_protocol::entity_data::Value;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct TextDisplayData {
    pub interpolation_delay: i32,
    pub interpolation_duration: i32,
    pub teleport_duration: i32,
    pub translation: [f32; 3],
    pub scale: [f32; 3],
    pub left_rotation: [f32; 4],
    pub right_rotation: [f32; 4],
    pub billboard: u8,
    pub brightness: i32,
    pub view_range: f32,
    pub shadow_radius: f32,
    pub shadow_strength: f32,
    pub width: f32,
    pub height: f32,
    pub glow_color: i32,
    #[serde(skip)]
    pub text: Text,
    pub line_width: i32,
    pub background: i32,
    pub opacity: i8,
    pub flags: u8,
}

impl Default for TextDisplayData {
    fn default() -> Self {
        Self {
            interpolation_delay: 0,
            interpolation_duration: 0,
            teleport_duration: 0,
            translation: [0.0; 3],
            scale: [1.0; 3],
            left_rotation: [0.0, 0.0, 0.0, 1.0],
            right_rotation: [0.0, 0.0, 0.0, 1.0],
            billboard: 0,
            brightness: -1,
            view_range: 1.0,
            shadow_radius: 0.0,
            shadow_strength: 1.0,
            width: 0.0,
            height: 0.0,
            glow_color: -1,
            text: Text::empty(),
            line_width: 200,
            background: 0x4000_0000,
            opacity: -1,
            flags: 0,
        }
    }
}

impl TextDisplayData {
    /// Omitted fields retain their previous values.
    pub fn apply(&mut self, values: &[(u8, i32, Value)]) {
        for (id, _, value) in values {
            match (id, value) {
                (8, Value::Int(v)) => self.interpolation_delay = *v,
                (9, Value::Int(v)) => self.interpolation_duration = *v,
                (10, Value::Int(v)) => self.teleport_duration = *v,
                (11, Value::Floats(v)) if v.len() == 3 => self.translation.copy_from_slice(v),
                (12, Value::Floats(v)) if v.len() == 3 => self.scale.copy_from_slice(v),
                (13, Value::Floats(v)) if v.len() == 4 => self.left_rotation.copy_from_slice(v),
                (14, Value::Floats(v)) if v.len() == 4 => self.right_rotation.copy_from_slice(v),
                (15, Value::Byte(v)) => self.billboard = *v as u8,
                (16, Value::Int(v)) => self.brightness = *v,
                (17, Value::Float(v)) => self.view_range = *v,
                (18, Value::Float(v)) => self.shadow_radius = *v,
                (19, Value::Float(v)) => self.shadow_strength = *v,
                (20, Value::Float(v)) => self.width = *v,
                (21, Value::Float(v)) => self.height = *v,
                (22, Value::Int(v)) => self.glow_color = *v,
                (23, Value::Text(v)) => self.text = crate::entity_nbt::name_text(v),
                (24, Value::Int(v)) => self.line_width = *v,
                (25, Value::Int(v)) => self.background = *v,
                (26, Value::Byte(v)) => self.opacity = *v,
                (27, Value::Byte(v)) => self.flags = *v as u8,
                _ => {}
            }
        }
    }
    pub fn alignment(&self) -> &'static str {
        if self.flags & 8 != 0 {
            "LEFT"
        } else if self.flags & 16 != 0 {
            "RIGHT"
        } else {
            "CENTER"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_updates_retain_values_and_bad_vectors_do_not_panic() {
        let mut d = TextDisplayData::default();
        d.apply(&[(24, 1, Value::Int(80)), (27, 0, Value::Byte(24))]);
        d.apply(&[(11, 0, Value::Floats(vec![1.0]))]);
        assert_eq!(d.line_width, 80);
        assert_eq!(d.alignment(), "LEFT");
        assert_eq!(d.translation, [0.0; 3]);
    }
}
