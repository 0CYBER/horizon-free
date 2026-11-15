//! An Xbox One pad's output reports, built by hand (docs/controllers.md "Xbox One output"): the
//! two handle motors and the two impulse-trigger motors, sent over hidapi ([`device`]) or, on
//! Windows, through Windows.Gaming.Input, which is the only API that reaches a wired pad's
//! triggers. Gilrs keeps reading the pad; this module only writes to it.
//!
//! The bytes are the ones SDL's `SDL_hidapi_xboxone.c` writes (`HIDAPI_DriverXboxOne_UpdateRumble`):
//! report 0x09 over USB, 0x0F turning the handle and the trigger motors on, then the four levels;
//! report 0x03 over Bluetooth, with the same four levels. Only Microsoft's own pads have the
//! trigger motors (SDL's `ControllerHasTriggerRumble`): on third-party Xbox pads they buzz.
//! On iOS the crate builds without hidapi, which has no backend there: [`Link`] never opens.
pub mod device;
#[cfg(windows)]
mod wgi;

#[cfg(not(target_os = "ios"))]
pub use device::{Error, Xbox};
pub use device::{Link, Transport};

/// Microsoft's vendor ID.
pub const MICROSOFT: u16 = 0x045e;

/// Microsoft's Xbox One and Series pads (SDL's `usb_ids.h` and `SDL_IsJoystickXboxOne`), the ones
/// with impulse triggers. Microsoft's other HID devices (keyboards, mice) share the vendor ID;
/// Xbox 360 pads have their own ids and no trigger motors.
pub const XBOX_ONE: [u16; 15] = [
    0x02d1, // Xbox One
    0x02dd, // Xbox One S, wired
    0x02e0, // Xbox One S, Bluetooth
    0x02e3, // Xbox One Elite
    0x02ea, // Xbox One S, Bluetooth
    0x02fd, // Xbox One S, Bluetooth (second revision)
    0x0b00, // Xbox Elite Series 2, wired
    0x0b05, // Xbox Elite Series 2, Bluetooth
    0x0b0a, // Xbox Adaptive Controller, wired
    0x0b0c, // Xbox Adaptive Controller, Bluetooth
    0x0b12, // Xbox Series X|S, wired
    0x0b13, // Xbox Series X|S, Bluetooth
    0x0b20, // Xbox One S, Bluetooth LE
    0x0b21, // Xbox Adaptive Controller, Bluetooth LE
    0x0b22, // Xbox Elite Series 2, Bluetooth LE
];

/// Whether a USB vendor / product pair is a Microsoft Xbox One pad.
pub fn is_xbox_one(vendor: u16, product: u16) -> bool {
    vendor == MICROSOFT && XBOX_ONE.contains(&product)
}

/// Which pad a [`Link`] writes to: its USB ids, and which of several identical pads it is (0 is
/// the first the system lists). Gilrs tells the game which pad the player is using; the writer
/// asks Windows.Gaming.Input or hidapi for that one, so a second controller can't take the first
/// one's output.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub vendor: u16,
    pub product: u16,
    pub index: usize,
}

impl Target {
    /// The first pad the output can drive, whatever its ids: what the example and older callers
    /// ask for. Only Microsoft's Xbox One pads qualify.
    pub const ANY: Target = Target {
        vendor: 0,
        product: 0,
        index: 0,
    };

    /// The pad with these ids, the `index`-th of its model in the order the system lists them.
    pub fn new(vendor: u16, product: u16, index: usize) -> Target {
        Target {
            vendor,
            product,
            index,
        }
    }

    /// Whether a pad with these ids is this target. [`Target::ANY`] matches every Xbox One pad.
    pub fn matches(self, vendor: u16, product: u16) -> bool {
        self == Target::ANY || (self.vendor == vendor && self.product == product)
    }

    /// Whether the Xbox One reports are the pad's own. Only Microsoft's pads are known to take
    /// them; another pad's four motors are reachable through Windows.Gaming.Input alone.
    pub fn xbox_reports(self) -> bool {
        is_xbox_one(self.vendor, self.product)
    }
}

/// The pad's four motors, 0..255: the left (heavy, low) and right (light, high) handles, then
/// the left and right triggers' impulse motors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motors {
    pub strong: u8,
    pub weak: u8,
    pub left_trigger: u8,
    pub right_trigger: u8,
}

impl Motors {
    /// All four stopped.
    pub const STOP: Motors = Motors {
        strong: 0,
        weak: 0,
        left_trigger: 0,
        right_trigger: 0,
    };

    /// The motors at these levels, 0..1 each: the handles' strong and weak, and the triggers'.
    pub fn levels(strong: f32, weak: f32, triggers: [f32; 2]) -> Motors {
        let level = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Motors {
            strong: level(strong),
            weak: level(weak),
            left_trigger: level(triggers[0]),
            right_trigger: level(triggers[1]),
        }
    }
}

pub const USB_REPORT_ID: u8 = 0x09;
pub const USB_REPORT_LEN: usize = 13;
pub const BT_REPORT_ID: u8 = 0x03;
pub const BT_REPORT_LEN: usize = 9;
/// The effect's duration, start delay and loop count, as SDL sends them: 0xFF steps of 10 ms, no
/// delay, and 0xEB.
const EFFECT: [u8; 3] = [0xff, 0x00, 0xeb];
/// `0x0F`: the report sets both handle motors and both trigger motors.
const MOTORS_ON: u8 = 0x0f;

/// The USB report: GIP's rumble command, 0x09, then 0x00, 0x00 (its client and sequence), the
/// payload's length 0x09, 0x00, 0x0F, the triggers' levels, the handles' levels, and the effect's
/// last three bytes.
pub fn usb(motors: Motors) -> [u8; USB_REPORT_LEN] {
    [
        USB_REPORT_ID,
        0x00,
        0x00,
        0x09,
        0x00,
        MOTORS_ON,
        motors.left_trigger,
        motors.right_trigger,
        motors.strong,
        motors.weak,
        EFFECT[0],
        EFFECT[1],
        EFFECT[2],
    ]
}

/// The Bluetooth report: 0x03, 0x0F, the four levels, then the effect's last three bytes.
pub fn bluetooth(motors: Motors) -> [u8; BT_REPORT_LEN] {
    [
        BT_REPORT_ID,
        MOTORS_ON,
        motors.left_trigger,
        motors.right_trigger,
        motors.strong,
        motors.weak,
        EFFECT[0],
        EFFECT[1],
        EFFECT[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usb_report_is_the_rumble_frame() {
        let motors = Motors {
            strong: 200,
            weak: 40,
            left_trigger: 90,
            right_trigger: 10,
        };
        assert_eq!(
            usb(motors),
            [
                0x09, 0x00, 0x00, 0x09, 0x00, 0x0f, 90, 10, 200, 40, 0xff, 0x00, 0xeb
            ]
        );
        assert_eq!(
            usb(Motors::STOP),
            [0x09, 0, 0, 9, 0, 0x0f, 0, 0, 0, 0, 0xff, 0, 0xeb]
        );
    }

    #[test]
    fn the_bluetooth_report_carries_the_same_four_levels() {
        let motors = Motors {
            strong: 200,
            weak: 40,
            left_trigger: 90,
            right_trigger: 10,
        };
        assert_eq!(
            bluetooth(motors),
            [0x03, 0x0f, 90, 10, 200, 40, 0xff, 0x00, 0xeb]
        );
        assert_eq!(
            bluetooth(Motors::STOP),
            [0x03, 0x0f, 0, 0, 0, 0, 0xff, 0x00, 0xeb]
        );
    }

    #[test]
    fn levels_scale_to_a_byte_and_clamp() {
        assert_eq!(Motors::levels(0.0, 0.0, [0.0, 0.0]), Motors::STOP);
        assert_eq!(
            Motors::levels(1.0, 0.5, [0.25, 2.0]),
            Motors {
                strong: 255,
                weak: 128,
                left_trigger: 64,
                right_trigger: 255,
            }
        );
        // Far off the scale, as a pulse's sum could be: still a byte.
        assert_eq!(Motors::levels(-1.0, 0.0, [0.0, 0.0]), Motors::STOP);
    }

    #[test]
    fn only_microsofts_xbox_pads_are_ours() {
        // Every id SDL calls an Xbox One pad, wired and wireless.
        for product in XBOX_ONE {
            assert!(is_xbox_one(MICROSOFT, product), "{product:04x}");
        }
        assert!(!is_xbox_one(MICROSOFT, 0x028e), "an Xbox 360 pad");
        assert!(!is_xbox_one(MICROSOFT, 0x02ff), "XGIP's software device");
        assert!(!is_xbox_one(0x054c, 0x0ce6), "a DualSense");
        assert!(!is_xbox_one(0x045e, 0x0001), "another Microsoft device");
    }

    #[test]
    fn a_target_names_one_pad_and_any_is_every_xbox_one() {
        let gamepad = Target::new(0x3537, 0x1022, 1);
        assert!(gamepad.matches(0x3537, 0x1022));
        assert!(!gamepad.matches(0x3537, 0x1023), "another GameSir model");
        assert!(!gamepad.matches(MICROSOFT, 0x02fd), "the Xbox One");
        // A third-party pad's reports are not the Xbox One ones: only WGI drives its four motors.
        assert!(!gamepad.xbox_reports());
        assert!(Target::new(MICROSOFT, 0x02fd, 0).xbox_reports());
        // The first Xbox One pad, as before targets existed.
        assert!(Target::ANY.matches(MICROSOFT, 0x02fd));
        assert!(!Target::ANY.xbox_reports());
    }
}
