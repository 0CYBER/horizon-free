// Copyright (C) 2004 by Sun Microsystems, Inc. All rights reserved.
//
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
//
// The scalbn helper also derives from:
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
//
// Rust adaptation of the original, permissively licensed netlib fdlibm
// e_exp.c (1.6, 2004-04-22), e_pow.c (1.5, 2004-04-22), and
// s_scalbn.c (1.3, 1995-01-18): https://www.netlib.org/fdlibm/.
// Preserve the C operation order: these are not the machine-dependent jmath paths.

use super::{from_high_word, high_word, low_word, set_high_word};

const LN2_HI: f64 = f64::from_bits(0x3fe6_2e42_fee0_0000);
const LN2_LO: f64 = f64::from_bits(0x3dea_39ef_3579_3c76);
const INV_LN2: f64 = f64::from_bits(0x3ff7_1547_652b_82fe);
const P1: f64 = f64::from_bits(0x3fc5_5555_5555_553e);
const P2: f64 = f64::from_bits(0xbf66_c16c_16be_bd93);
const P3: f64 = f64::from_bits(0x3f11_566a_af25_de2c);
const P4: f64 = f64::from_bits(0xbebb_bd41_c5d2_6bf1);
const P5: f64 = f64::from_bits(0x3e66_3769_72be_a4d0);
const TWO_M1000: f64 = f64::from_bits(0x0170_0000_0000_0000);
const OVERFLOW_THRESHOLD: f64 = f64::from_bits(0x4086_2e42_fefa_39ef);
const UNDERFLOW_THRESHOLD: f64 = f64::from_bits(0xc087_4910_d52d_3051);

/// StrictMath-exact exponential, linked into native and wasm guests without host calls.
pub fn exp(x: f64) -> f64 {
    let mut x = x;
    let hx = high_word(x) as u32;
    let negative = (hx >> 31) != 0;
    let hx = hx & 0x7fff_ffff;
    if hx >= 0x4086_2e42 {
        if hx >= 0x7ff0_0000 {
            if ((hx & 0x000f_ffff) | low_word(x)) != 0 {
                return x + x;
            }
            return if negative { 0.0 } else { x };
        }
        if x > OVERFLOW_THRESHOLD {
            return f64::INFINITY;
        }
        if x < UNDERFLOW_THRESHOLD {
            return 0.0;
        }
    }

    let (k, hi, lo);
    if hx > 0x3fd6_2e42 {
        if hx < 0x3ff0_a2b2 {
            hi = x - if negative { -LN2_HI } else { LN2_HI };
            lo = if negative { -LN2_LO } else { LN2_LO };
            k = if negative { -1 } else { 1 };
        } else {
            k = (INV_LN2 * x + if negative { -0.5 } else { 0.5 }) as i32;
            let t = f64::from(k);
            hi = x - t * LN2_HI;
            lo = t * LN2_LO;
        }
        x = hi - lo;
    } else if hx < 0x3e30_0000 {
        // fdlibm's huge+x>1 branch always succeeds for this finite range.
        return 1.0 + x;
    } else {
        k = 0;
        hi = 0.0;
        lo = 0.0;
    }

    let t = x * x;
    let c = x - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    }
    let y = 1.0 - ((lo - (x * c) / (2.0 - c)) - hi);
    if k >= -1021 {
        set_high_word(y, high_word(y) + (k << 20))
    } else {
        set_high_word(y, high_word(y) + ((k + 1000) << 20)) * TWO_M1000
    }
}

const BP: [f64; 2] = [1.0, 1.5];
const DP_H: [f64; 2] = [0.0, f64::from_bits(0x3fe2_b803_4000_0000)];
const DP_L: [f64; 2] = [0.0, f64::from_bits(0x3e4c_fdeb_43cf_d006)];
const TWO53: f64 = f64::from_bits(0x4340_0000_0000_0000);
const L1: f64 = f64::from_bits(0x3fe3_3333_3333_3303);
const L2: f64 = f64::from_bits(0x3fdb_6db6_db6f_abff);
const L3: f64 = f64::from_bits(0x3fd5_5555_518f_264d);
const L4: f64 = f64::from_bits(0x3fd1_7460_a91d_4101);
const L5: f64 = f64::from_bits(0x3fcd_864a_93c9_db65);
const L6: f64 = f64::from_bits(0x3fca_7e28_4a45_4eef);
const LG2: f64 = f64::from_bits(0x3fe6_2e42_fefa_39ef);
const LG2_H: f64 = f64::from_bits(0x3fe6_2e43_0000_0000);
const LG2_L: f64 = f64::from_bits(0xbe20_5c61_0ca8_6c39);
const OVT: f64 = f64::from_bits(0x3c97_1547_652b_82fe);
const CP: f64 = f64::from_bits(0x3fee_c709_dc3a_03fd);
const CP_H: f64 = f64::from_bits(0x3fee_c709_e000_0000);
const CP_L: f64 = f64::from_bits(0xbe3e_2fe0_145b_01f5);
const INV_LN2_H: f64 = f64::from_bits(0x3ff7_1547_6000_0000);
const INV_LN2_L: f64 = f64::from_bits(0x3e54_ae0b_f85d_df44);
const ONE_THIRD: f64 = f64::from_bits(0x3fd5_5555_5555_5555);

/// StrictMath-exact power, linked into native and wasm guests without host calls.
pub fn pow(x: f64, y: f64) -> f64 {
    let hx = high_word(x);
    let lx = low_word(x);
    let hy = high_word(y);
    let ly = low_word(y);
    let mut ix = hx & 0x7fff_ffff;
    let iy = hy & 0x7fff_ffff;

    if (iy as u32 | ly) == 0 {
        return 1.0;
    }
    if ix > 0x7ff0_0000
        || (ix == 0x7ff0_0000 && lx != 0)
        || iy > 0x7ff0_0000
        || (iy == 0x7ff0_0000 && ly != 0)
    {
        return x + y;
    }

    // For negative x: 0 means non-integer y, 1 odd, 2 even.
    let mut yisint = 0;
    if hx < 0 {
        if iy >= 0x4340_0000 {
            yisint = 2;
        } else if iy >= 0x3ff0_0000 {
            let k = (iy >> 20) - 0x3ff;
            if k > 20 {
                let j = ly >> (52 - k);
                if (j << (52 - k)) == ly {
                    yisint = 2 - (j & 1) as i32;
                }
            } else if ly == 0 {
                let j = iy >> (20 - k);
                if (j << (20 - k)) == iy {
                    yisint = 2 - (j & 1);
                }
            }
        }
    }

    if ly == 0 {
        if iy == 0x7ff0_0000 {
            if ix == 0x3ff0_0000 && lx == 0 {
                return f64::NAN;
            } else if ix >= 0x3ff0_0000 {
                return if hy >= 0 { y } else { 0.0 };
            } else {
                return if hy < 0 { -y } else { 0.0 };
            }
        }
        if iy == 0x3ff0_0000 {
            return if hy < 0 { 1.0 / x } else { x };
        }
        if hy == 0x4000_0000 {
            return x * x;
        }
        if hy == 0x3fe0_0000 && hx >= 0 {
            // IEEE-754 square root is correctly rounded on every supported target.
            return x.sqrt();
        }
    }

    let mut ax = x.abs();
    if lx == 0 && (ix == 0x7ff0_0000 || ix == 0 || ix == 0x3ff0_0000) {
        let mut z = ax;
        if hy < 0 {
            z = 1.0 / z;
        }
        if hx < 0 {
            if ix == 0x3ff0_0000 && yisint == 0 {
                z = f64::NAN;
            } else if yisint == 1 {
                z = -z;
            }
        }
        return z;
    }

    if hx < 0 && yisint == 0 {
        return f64::NAN;
    }
    let sign = if hx < 0 && yisint == 1 { -1.0 } else { 1.0 };

    let (t1, t2);
    if iy > 0x41e0_0000 {
        // Huge |y|: unless x is extremely close to one, overflow/underflow.
        if iy > 0x43f0_0000 {
            if ix <= 0x3fef_ffff {
                return if hy < 0 { f64::INFINITY } else { 0.0 };
            }
            if ix >= 0x3ff0_0000 {
                return if hy > 0 { f64::INFINITY } else { 0.0 };
            }
        }
        if ix < 0x3fef_ffff {
            return sign * if hy < 0 { f64::INFINITY } else { 0.0 };
        }
        if ix > 0x3ff0_0000 {
            return sign * if hy > 0 { f64::INFINITY } else { 0.0 };
        }
        let t = ax - 1.0;
        let w = (t * t) * (0.5 - t * (ONE_THIRD - t * 0.25));
        let u = INV_LN2_H * t;
        let v = t * INV_LN2_L - w * INV_LN2;
        t1 = clear_low_word(u + v);
        t2 = v - (t1 - u);
    } else {
        let mut n = 0;
        if ix < 0x0010_0000 {
            ax *= TWO53;
            n -= 53;
            ix = high_word(ax);
        }
        n += (ix >> 20) - 0x3ff;
        let j = ix & 0x000f_ffff;
        ix = j | 0x3ff0_0000;
        let k;
        if j <= 0x3988e {
            k = 0;
        } else if j < 0xbb67a {
            k = 1;
        } else {
            k = 0;
            n += 1;
            ix -= 0x0010_0000;
        }
        ax = set_high_word(ax, ix);

        // ss = s_h+s_l = (x-bp[k])/(x+bp[k]).
        let u = ax - BP[k];
        let v = 1.0 / (ax + BP[k]);
        let ss = u * v;
        let s_h = clear_low_word(ss);
        let t_h = from_high_word(((ix >> 1) | 0x2000_0000) + 0x0008_0000 + ((k as i32) << 18));
        let t_l = ax - (t_h - BP[k]);
        let s_l = v * ((u - s_h * t_h) - s_h * t_l);
        let s2 = ss * ss;
        let mut r = s2 * s2 * (L1 + s2 * (L2 + s2 * (L3 + s2 * (L4 + s2 * (L5 + s2 * L6)))));
        r += s_l * (s_h + ss);
        let s2 = s_h * s_h;
        let t_h = clear_low_word(3.0 + s2 + r);
        let t_l = r - ((t_h - 3.0) - s2);
        let u = s_h * t_h;
        let v = s_l * t_h + t_l * ss;
        let p_h = clear_low_word(u + v);
        let p_l = v - (p_h - u);
        let z_h = CP_H * p_h;
        let z_l = CP_L * p_h + p_l * CP + DP_L[k];
        let t = f64::from(n);
        t1 = clear_low_word(((z_h + z_l) + DP_H[k]) + t);
        t2 = z_l - (((t1 - t) - DP_H[k]) - z_h);
    }

    // Multiply the split y and log2(x) without losing their low pieces.
    let y1 = clear_low_word(y);
    let p_l = (y - y1) * t1 + y * t2;
    let mut p_h = y1 * t1;
    let z = p_l + p_h;
    let j = high_word(z);
    let i = low_word(z);
    if j >= 0x4090_0000 {
        if j != 0x4090_0000 || i != 0 || p_l + OVT > z - p_h {
            return sign * f64::INFINITY;
        }
    } else if (j & 0x7fff_ffff) >= 0x4090_cc00
        && (j as u32 != 0xc090_cc00 || i != 0 || p_l <= z - p_h)
    {
        return sign * 0.0;
    }

    // Compute 2**(p_h+p_l), reducing the exponent to at most 1/2.
    let i = j & 0x7fff_ffff;
    let mut k = (i >> 20) - 0x3ff;
    let mut n = 0;
    if i > 0x3fe0_0000 {
        n = j + (0x0010_0000 >> (k + 1));
        k = ((n & 0x7fff_ffff) >> 20) - 0x3ff;
        let t = from_high_word(n & !(0x000f_ffff >> k));
        n = ((n & 0x000f_ffff) | 0x0010_0000) >> (20 - k);
        if j < 0 {
            n = -n;
        }
        p_h -= t;
    }
    let t = clear_low_word(p_l + p_h);
    let u = t * LG2_H;
    let v = (p_l - (t - p_h)) * LG2 + t * LG2_L;
    let z = u + v;
    let w = v - (z - u);
    let t = z * z;
    let t1 = z - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    let r = (z * t1) / (t1 - 2.0) - (w + z * w);
    let z = 1.0 - (r - z);
    let j = high_word(z) + (n << 20);
    let z = if (j >> 20) <= 0 {
        scalbn(z, n)
    } else {
        set_high_word(z, j)
    };
    sign * z
}

fn clear_low_word(x: f64) -> f64 {
    from_high_word(high_word(x))
}

// fdlibm s_scalbn.c, with signed overflow/underflow expressed as IEEE values.
fn scalbn(mut x: f64, n: i32) -> f64 {
    const TWO54: f64 = f64::from_bits(0x4350_0000_0000_0000);
    const TWO_M54: f64 = f64::from_bits(0x3c90_0000_0000_0000);
    let mut hx = high_word(x);
    let mut k = (hx & 0x7ff0_0000) >> 20;
    if k == 0 {
        if (low_word(x) | (hx as u32 & 0x7fff_ffff)) == 0 {
            return x;
        }
        x *= TWO54;
        hx = high_word(x);
        k = ((hx & 0x7ff0_0000) >> 20) - 54;
        if n < -50000 {
            return 1.0e-300 * x;
        }
    }
    if k == 0x7ff {
        return x + x;
    }
    // i64 also covers the original C helper's extreme-i32 exponent cases.
    let k = i64::from(k) + i64::from(n);
    if k > 0x7fe {
        return f64::INFINITY.copysign(x);
    }
    if k > 0 {
        return set_high_word(x, (hx & 0x800f_ffffu32 as i32) | ((k as i32) << 20));
    }
    if k <= -54 {
        return 0.0f64.copysign(x);
    }
    set_high_word(x, (hx & 0x800f_ffffu32 as i32) | (((k + 54) as i32) << 20)) * TWO_M54
}
