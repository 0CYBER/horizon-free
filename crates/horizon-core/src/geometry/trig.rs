//! Vanilla's quantized float sine table with Java double-to-long indexing.
use std::sync::OnceLock;

pub const DEG_TO_RAD: f32 = f32::from_bits(0x3c8e_fa35);
pub const RAD_TO_DEG: f32 = f32::from_bits(0x4265_2ee0);
const SCALE: f64 = 10430.378350470453;
static SINE: OnceLock<Box<[f32]>> = OnceLock::new();

/// Generated from the 26.3 table formula, with no bundled vanilla artifact.
/// Differential validation checks all 65,536 entries on the tested platform.
pub fn sine_table() -> &'static [f32] {
    SINE.get_or_init(|| {
        (0..65536)
            .map(|i| (i as f64 / SCALE).sin() as f32)
            .collect::<Vec<_>>()
            .into_boxed_slice()
    })
}
pub fn sin(angle: f64) -> f32 {
    sine_table()[((angle * SCALE) as i64 & 65535) as usize]
}
pub fn cos(angle: f64) -> f32 {
    sine_table()[((angle * SCALE + 16384.0) as i64 & 65535) as usize]
}

/// `Mth.atan2`: vanilla's table-driven arctangent.
pub fn atan2(mut y: f64, mut x: f64) -> f64 {
    static TABLES: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();
    let (asin, cos) = TABLES.get_or_init(|| {
        (0..257)
            .map(|i| {
                let a = fdlibm_asin(f64::from(i) / 256.0);
                // HotSpot's x86-64 Math.cos intrinsic rounds entry 61 one ulp
                // above the correctly rounded value the platform cos gives.
                let c = if i == 61 {
                    f64::from_bits(0x3fef_140a_0a08_6af1)
                } else {
                    a.cos()
                };
                (a, c)
            })
            .unzip()
    });
    let frac_bias = f64::from_bits(4_805_340_802_404_319_232);
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }
    let negative_y = y < 0.0;
    if negative_y {
        y = -y;
    }
    let negative_x = x < 0.0;
    if negative_x {
        x = -x;
    }
    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }
    // Mth.fastInvSqrt
    let half = 0.5 * d2;
    let mut r = f64::from_bits((6_910_469_410_427_058_090i64 - (d2.to_bits() as i64 >> 1)) as u64);
    r *= 1.5 - half * r * r;
    x *= r;
    y *= r;
    let yp = frac_bias + y;
    let index = yp.to_bits() as i32 as usize;
    let s_phi = yp - frac_bias;
    let sd = y * cos[index] - x * s_phi;
    let mut theta = asin[index] + (6.0 + sd * sd) * sd * 0.166_666_666_666_666_66;
    if steep {
        theta = std::f64::consts::FRAC_PI_2 - theta;
    }
    if negative_x {
        theta = std::f64::consts::PI - theta;
    }
    if negative_y {
        theta = -theta;
    }
    theta
}
/// `(float)(Mth.atan2(y, x) * 180.0F / (float)Math.PI)` as the AI controls
/// compute it: the compiled code multiplies by the folded float constant
/// `Mth.RAD_TO_DEG` widened to double.
pub fn atan2_degrees(y: f64, x: f64) -> f32 {
    (atan2(y, x) * f64::from(RAD_TO_DEG)) as f32
}

/// `Mth.sin` / `Mth.cos` on a float, as the models call them.
pub trait Mth {
    fn msin(self) -> f32;
    fn mcos(self) -> f32;
}
impl Mth for f32 {
    fn msin(self) -> f32 {
        sin(f64::from(self))
    }
    fn mcos(self) -> f32 {
        cos(f64::from(self))
    }
}

/// fdlibm `__ieee754_asin`, as `StrictMath.asin` (and so `Math.asin`) computes it.
#[allow(clippy::excessive_precision, clippy::approx_constant, clippy::eq_op)] // fdlibm's constants and NaN, verbatim.
pub fn fdlibm_asin(x: f64) -> f64 {
    const PIO2_HI: f64 = 1.570_796_326_794_896_558_00e+00;
    const PIO2_LO: f64 = 6.123_233_995_736_766_035_87e-17;
    const PIO4_HI: f64 = 7.853_981_633_974_482_789_99e-01;
    const PS0: f64 = 1.666_666_666_666_666_574_15e-01;
    const PS1: f64 = -3.255_658_186_224_009_154_05e-01;
    const PS2: f64 = 2.012_125_321_348_629_258_81e-01;
    const PS3: f64 = -4.005_553_450_067_941_140_27e-02;
    const PS4: f64 = 7.915_349_942_898_145_321_76e-04;
    const PS5: f64 = 3.479_331_075_960_211_675_70e-05;
    const QS1: f64 = -2.403_394_911_734_414_218_78e+00;
    const QS2: f64 = 2.020_945_760_233_505_694_71e+00;
    const QS3: f64 = -6.882_839_716_054_532_930_30e-01;
    const QS4: f64 = 7.703_815_055_590_193_527_91e-02;
    let bits = x.to_bits();
    let hx = (bits >> 32) as i32;
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        if ((ix - 0x3ff0_0000) as u32 | bits as u32) == 0 {
            return x * PIO2_HI + x * PIO2_LO;
        }
        return (x - x) / (x - x);
    } else if ix < 0x3fe0_0000 {
        if ix < 0x3e40_0000 && 1.0e300 + x > 1.0 {
            return x;
        }
        let t = x * x;
        let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
        let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
        let w = p / q;
        return x + x * w;
    }
    let w = 1.0 - x.abs();
    let t = w * 0.5;
    let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
    let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
    let s = t.sqrt();
    let t = if ix >= 0x3fef_3333 {
        let w = p / q;
        PIO2_HI - (2.0 * (s + s * w) - PIO2_LO)
    } else {
        let w = f64::from_bits(s.to_bits() & 0xffff_ffff_0000_0000);
        let c = (t - w * w) / (s + w);
        let r = p / q;
        let p = 2.0 * s * r - (PIO2_LO - 2.0 * c);
        let q = PIO4_HI - 2.0 * w;
        PIO4_HI - (p - q)
    };
    if hx > 0 { t } else { -t }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Values the 26.3 client computes on x86-64 HotSpot.
    #[test]
    fn atan2_matches_the_client_tables() {
        for (y, x, r) in [
            (
                0x402f_3b4b_8041_4df1_u64,
                0xc018_53ca_369b_9e91_u64,
                0x3fff_1334_9f9d_e798_u64,
            ),
            (
                0xc025_d316_35f2_9de8,
                0xc018_fb4a_7ec4_3ba8,
                0xc000_b996_6547_a498,
            ),
            (
                0xc026_5d9c_2b5a_472d,
                0xc030_954d_c2dc_3d10,
                0xc004_62e7_82d4_c97e,
            ),
        ] {
            assert_eq!(atan2(f64::from_bits(y), f64::from_bits(x)).to_bits(), r);
        }
        assert_eq!(fdlibm_asin(60.0 / 256.0).to_bits(), 0x3fce_481c_0fce_7134);
        assert_eq!(fdlibm_asin(61.0 / 256.0).to_bits(), 0x3fce_cbd7_5b6c_d8f0);
    }
    #[test]
    fn long_narrowing_and_quadrants() {
        assert_eq!(sin(0.0).to_bits(), 0.0f32.to_bits());
        assert_eq!(sin(f64::NAN), sin(0.0));
        assert_eq!(sin(f64::INFINITY), sine_table()[65535]);
        assert_eq!(sin(f64::NEG_INFINITY), sine_table()[0]);
        assert_eq!(cos(0.0), 1.0);
        assert_eq!(sin(std::f64::consts::FRAC_PI_2), 1.0);
    }
}
