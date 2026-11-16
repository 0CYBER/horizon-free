use super::Poses;

pub(super) fn apply(poses: &mut Poses, angles: [[u32; 3]; 2]) {
    for (name, angles) in ["right_arm", "left_arm"].into_iter().zip(angles) {
        for (axis, bits) in angles.into_iter().enumerate() {
            let angle = f32::from_bits(bits);
            if angle.is_finite() {
                poses.get(name)[3 + axis] = angle;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nonfinite_components_preserve_the_original_angles() {
        let mut poses = Poses::default();
        poses
            .map
            .insert(crate::mobs::anim::key("right_arm"), [0.25; 9]);
        poses
            .map
            .insert(crate::mobs::anim::key("left_arm"), [0.5; 9]);
        apply(
            &mut poses,
            [
                [
                    f32::NAN.to_bits(),
                    f32::INFINITY.to_bits(),
                    (-0.8f32).to_bits(),
                ],
                [f32::NEG_INFINITY.to_bits(); 3],
            ],
        );
        assert_eq!(&poses.get("right_arm")[3..6], &[0.25, 0.25, -0.8]);
        assert_eq!(&poses.get("left_arm")[3..6], &[0.5; 3]);
    }
}
