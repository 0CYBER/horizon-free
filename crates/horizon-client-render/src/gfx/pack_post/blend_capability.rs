//! Required attachment blending is a device contract, checked before pipeline fallback.

pub(super) fn check(required: bool, flags: wgpu::DownlevelFlags) -> Result<(), super::Refused> {
    if required && !flags.contains(wgpu::DownlevelFlags::INDEPENDENT_BLEND) {
        return Err(super::Refused {
            reason: "the pack requires `PER_BUFFER_BLENDING`, but this GPU does not support independent attachment blending".into(),
            notes: Vec::new(),
        });
    }
    Ok(())
}
