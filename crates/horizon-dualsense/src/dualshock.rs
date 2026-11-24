//! A DualShock 4's output: the two handles and the lightbar, written as SDL's
//! `SDL_hidapi_ps4.c` writes them (docs/controllers.md, "Xbox One and DualShock 4 output"). The pad
//! has no trigger motors and no HD haptics, so the trigger levels are dropped.
use crate::xbox::Motors;

/// Sony's vendor ID (the same as the DualSense's, in [`crate::SONY`]).
pub const SONY: u16 = crate::SONY;
/// The DualShock 4's product IDs: the launch pad (CUH-ZCT1) and the slim one (CUH-ZCT2). The
/// wireless adapter's id (0x0ba0) is left out: it takes reports of its own.
pub const DUALSHOCK_IDS: [u16; 2] = [0x05c4, 0x09cc];

/// Whether a USB vendor / product pair is a DualShock 4.
pub fn is_dualshock(vendor: u16, product: u16) -> bool {
    vendor == SONY && DUALSHOCK_IDS.contains(&product)
}

/// The effect block's first byte marks which of its fields the report sets.
const RUMBLE: u8 = 0x01;
const LIGHT: u8 = 0x02;
/// SDL's Bluetooth reports lead with `0xC0 | report_interval`; 4 ms is its default.
const BT_MAGIC: u8 = 0xc0 | 4;

pub const USB_REPORT_ID: u8 = 0x05;
pub const USB_REPORT_LEN: usize = 32;
pub const BT_REPORT_ID: u8 = 0x11;
pub const BT_REPORT_LEN: usize = 78;
/// Both reports carry the effect at this offset (report ID included).
const USB_EFFECT: usize = 4;
const BT_EFFECT: usize = 6;

/// The mask for a report: the handles always, the lightbar when the game has a colour for it.
fn mask(light: Option<[u8; 3]>) -> u8 {
    RUMBLE | if light.is_some() { LIGHT } else { 0 }
}

/// The USB report: 0x05, the mask, then the effect at offset 4 — the light motor, the heavy one,
/// 0..255, and the lightbar when `light` is set, its red, green and blue (the flash delays stay
/// zero). The trigger motors' levels are dropped: the pad has none.
pub fn usb(motors: Motors, light: Option<[u8; 3]>) -> [u8; USB_REPORT_LEN] {
    let mut r = [0; USB_REPORT_LEN];
    r[0] = USB_REPORT_ID;
    r[1] = mask(light);
    r[USB_EFFECT] = motors.weak;
    r[USB_EFFECT + 1] = motors.strong;
    if let Some(rgb) = light {
        r[USB_EFFECT + 2..USB_EFFECT + 5].copy_from_slice(&rgb);
    }
    r
}

/// The Bluetooth report: 0x11, 0xC0 | the report interval, the mask at 3, the effect at 6, and
/// the CRC-32 of the seed byte 0xA2 and the first 74 bytes, little-endian.
pub fn bluetooth(motors: Motors, light: Option<[u8; 3]>) -> [u8; BT_REPORT_LEN] {
    let mut r = [0; BT_REPORT_LEN];
    r[0] = BT_REPORT_ID;
    r[1] = BT_MAGIC;
    r[3] = mask(light);
    r[BT_EFFECT] = motors.weak;
    r[BT_EFFECT + 1] = motors.strong;
    if let Some(rgb) = light {
        r[BT_EFFECT + 2..BT_EFFECT + 5].copy_from_slice(&rgb);
    }
    let crc = crate::bt_crc(&r[..BT_REPORT_LEN - 4]);
    r[BT_REPORT_LEN - 4..].copy_from_slice(&crc.to_le_bytes());
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sonys_dualshocks_are_ours() {
        assert!(is_dualshock(SONY, 0x05c4));
        assert!(is_dualshock(SONY, 0x09cc));
        assert!(!is_dualshock(SONY, 0x0ce6), "a DualSense");
        assert!(!is_dualshock(0x045e, 0x09cc), "another maker's pad");
    }

    /// The layout SDL sends over USB: mask 0x01 at [1], the light motor at [4], the heavy at [5],
    /// and nothing in the 27 bytes after (the lightbar is not touched).
    #[test]
    fn the_usb_report_carries_the_two_handles() {
        let motors = Motors {
            strong: 200,
            weak: 40,
            left_trigger: 255,
            right_trigger: 255,
        };
        let mut expected = [0; USB_REPORT_LEN];
        expected[0] = 0x05;
        expected[1] = 0x01;
        expected[4] = 40;
        expected[5] = 200;
        assert_eq!(usb(motors, None), expected);
    }

    /// The lightbar's bytes follow the handles in the same effect block, and the mask says so:
    /// SDL's `DS4EffectsState_t` is rumbleRight, rumbleLeft, ledRed, ledGreen, ledBlue, delayOn,
    /// delayOff.
    #[test]
    fn the_lightbar_rides_the_same_effect_block() {
        let motors = Motors {
            strong: 200,
            weak: 40,
            ..Motors::STOP
        };
        let light = Some([0x11, 0x9d, 0x2e]);
        let mut expected = [0; USB_REPORT_LEN];
        expected[0] = 0x05;
        expected[1] = 0x03;
        expected[4] = 40;
        expected[5] = 200;
        expected[6..9].copy_from_slice(&[0x11, 0x9d, 0x2e]);
        assert_eq!(usb(motors, light), expected);

        let bt = bluetooth(motors, light);
        assert_eq!(
            &bt[..11],
            &[
                0x11, 0xc4, 0x00, 0x03, 0x00, 0x00, 40, 200, 0x11, 0x9d, 0x2e
            ]
        );
    }

    /// The layout SDL sends over Bluetooth, with its CRC of the seed 0xA2 and the first 74
    /// bytes (the expected CRC is zlib's crc32 over the same bytes, computed apart).
    #[test]
    fn the_bluetooth_report_ends_with_the_seeded_crc() {
        let report = bluetooth(
            Motors {
                strong: 200,
                weak: 40,
                left_trigger: 0,
                right_trigger: 0,
            },
            None,
        );
        assert_eq!(&report[..8], &[0x11, 0xc4, 0x00, 0x01, 0x00, 0x00, 40, 200]);
        assert!(report[8..74].iter().all(|&b| b == 0));
        assert_eq!(report[74..], [0x30, 0xb3, 0x23, 0xf6]);
    }

    /// The CRC covers the lightbar too: zlib's crc32 of the seed 0xA2 and the first 74 bytes of
    /// this report, computed apart.
    #[test]
    fn the_bluetooth_light_report_keeps_its_crc() {
        let report = bluetooth(
            Motors {
                strong: 200,
                weak: 40,
                ..Motors::STOP
            },
            Some([0x11, 0x9d, 0x2e]),
        );
        assert_eq!(report[74..], [0xad, 0x90, 0x17, 0xbf]);
    }

    /// Stopped is zero where the motors are: a release, not a report leaving them on.
    #[test]
    fn stopped_writes_zeroes_where_the_motors_are() {
        let usb = usb(Motors::STOP, None);
        assert_eq!((usb[4], usb[5]), (0, 0));
        let bt = bluetooth(Motors::STOP, None);
        assert_eq!((bt[6], bt[7]), (0, 0));
    }
}
