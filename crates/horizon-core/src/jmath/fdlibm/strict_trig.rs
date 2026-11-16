// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
//
// k_tan.c:
// Copyright 2004 Sun Microsystems, Inc. All Rights Reserved.
//
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
//
// Rust adaptation of the original https://www.netlib.org/fdlibm/ sources:
// s_sin.c, s_cos.c, s_tan.c, k_sin.c, k_cos.c, k_tan.c,
// e_rem_pio2.c and k_rem_pio2.c. The large reduction kernel is specialized
// to prec=2, the precision used by e_rem_pio2.c for double arguments.

//! StrictMath trigonometry, with unfused fdlibm operation ordering and the
//! full 24-bit-chunk argument reduction for every finite double.

use super::{
    C1, C2, C3, C4, C5, C6, INVPIO2, NPIO2_HW, PIO2_1, PIO2_1T, PIO2_2, PIO2_2T, PIO2_3, PIO2_3T,
    S1, S2, S3, S4, S5, S6, from_high_word, high_word, low_word, set_high_word,
};

const TWO24: f64 = 16_777_216.0;
const TWON24: f64 = f64::from_bits(0x3e70_0000_0000_0000);

// Successive 24-bit digits of 2/pi, including all digits needed by f64::MAX.
const TWO_OVER_PI: [i32; 66] = [
    0xa2f983, 0x6e4e44, 0x1529fc, 0x2757d1, 0xf534dd, 0xc0db62, 0x95993c, 0x439041, 0xfe5163,
    0xabdebb, 0xc561b7, 0x246e3a, 0x424dd2, 0xe00649, 0x2eea09, 0xd1921c, 0xfe1deb, 0x1cb129,
    0xa73ee8, 0x8235f5, 0x2ebb44, 0x84e99c, 0x7026b4, 0x5f7e41, 0x3991d6, 0x398353, 0x39f49c,
    0x845f8b, 0xbdf928, 0x3b1ff8, 0x97ffde, 0x05980f, 0xef2f11, 0x8b5a0a, 0x6d1f6d, 0x367ecf,
    0x27cb09, 0xb74f46, 0x3f669e, 0x5fea2d, 0x7527ba, 0xc7ebe5, 0xf17b3d, 0x0739f7, 0x8a5292,
    0xea6bfb, 0x5fb11f, 0x8d5d08, 0x560330, 0x46fc7b, 0x6babf0, 0xcfbc20, 0x9af436, 0x1da9e3,
    0x91615e, 0xe61b08, 0x659985, 0x5f14a0, 0x68408d, 0xffd880, 0x4d7327, 0x310606, 0x1556ca,
    0x73a8c9, 0x60e27b, 0xc08c6b,
];

// pi/2 split into 24-bit pieces (k_rem_pio2.c's PIo2).
const PIO2_CHUNKS: [f64; 8] = [
    f64::from_bits(0x3ff9_21fb_4000_0000),
    f64::from_bits(0x3e74_442d_0000_0000),
    f64::from_bits(0x3cf8_4698_8000_0000),
    f64::from_bits(0x3b78_cc51_6000_0000),
    f64::from_bits(0x39f0_1b83_8000_0000),
    f64::from_bits(0x387a_2520_4000_0000),
    f64::from_bits(0x36e3_8222_8000_0000),
    f64::from_bits(0x3569_f31d_0000_0000),
];

const T: [f64; 13] = [
    f64::from_bits(0x3fd5_5555_5555_5563),
    f64::from_bits(0x3fc1_1111_1110_fe7a),
    f64::from_bits(0x3fab_a1ba_1bb3_41fe),
    f64::from_bits(0x3f96_64f4_8406_d637),
    f64::from_bits(0x3f82_26e3_e96e_8493),
    f64::from_bits(0x3f6d_6d22_c956_0328),
    f64::from_bits(0x3f57_dbc8_fee0_8315),
    f64::from_bits(0x3f43_44d8_f2f2_6501),
    f64::from_bits(0x3f30_26f7_1a8d_1068),
    f64::from_bits(0x3f14_7e88_a037_92a6),
    f64::from_bits(0x3f12_b80f_32f0_a7e9),
    f64::from_bits(0xbef3_75cb_db60_5373),
    f64::from_bits(0x3efb_2a70_74bf_7ad4),
];
const PIO4: f64 = f64::from_bits(0x3fe9_21fb_5444_2d18);
const PIO4_LO: f64 = f64::from_bits(0x3c81_a626_3314_5c07);

/// StrictMath-exact sine, linked into the caller without a host call.
#[allow(clippy::eq_op)] // fdlibm: x - x is NaN for an infinite or NaN input, verbatim.
pub fn sin(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return sin_kernel(x, 0.0, false);
    }
    if ix >= 0x7ff0_0000 {
        return x - x;
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => sin_kernel(y0, y1, true),
        1 => cos_kernel(y0, y1),
        2 => -sin_kernel(y0, y1, true),
        _ => -cos_kernel(y0, y1),
    }
}

/// StrictMath-exact cosine, linked into the caller without a host call.
#[allow(clippy::eq_op)] // fdlibm: x - x is NaN for an infinite or NaN input, verbatim.
pub fn cos(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return cos_kernel(x, 0.0);
    }
    if ix >= 0x7ff0_0000 {
        return x - x;
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => cos_kernel(y0, y1),
        1 => -sin_kernel(y0, y1, true),
        2 => -cos_kernel(y0, y1),
        _ => sin_kernel(y0, y1, true),
    }
}

/// StrictMath-exact tangent, linked into the caller without a host call.
#[allow(clippy::eq_op)] // fdlibm: x - x is NaN for an infinite or NaN input, verbatim.
pub fn tan(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return tan_kernel(x, 0.0, 1);
    }
    if ix >= 0x7ff0_0000 {
        return x - x;
    }
    let (n, y0, y1) = rem_pio2(x);
    tan_kernel(y0, y1, 1 - ((n & 1) << 1))
}

fn sin_kernel(x: f64, y: f64, reduced: bool) -> f64 {
    if high_word(x) & 0x7fff_ffff < 0x3e40_0000 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    if !reduced {
        x + v * (S1 + z * r)
    } else {
        x - ((z * (0.5 * y - v * r) - y) - v * S1)
    }
}

fn cos_kernel(x: f64, y: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 {
        return 1.0;
    }
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    if ix < 0x3fd3_3333 {
        1.0 - (0.5 * z - (z * r - x * y))
    } else {
        let qx = if ix > 0x3fe9_0000 {
            0.28125
        } else {
            from_high_word(ix - 0x0020_0000)
        };
        let hz = 0.5 * z - qx;
        let a = 1.0 - qx;
        a - (hz - (z * r - x * y))
    }
}

fn tan_kernel(mut x: f64, mut y: f64, iy: i32) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix < 0x3e30_0000 {
        if (ix as u32 | low_word(x)) == 0 && iy == -1 {
            return 1.0 / set_high_word(x, ix);
        }
        if iy == 1 {
            return x;
        }
        // Compute -1/(x+y), keeping the low part of the denominator.
        let w = x + y;
        let z = f64::from_bits(w.to_bits() & 0xffff_ffff_0000_0000);
        let v = y - (z - x);
        let a = -1.0 / w;
        let t = f64::from_bits(a.to_bits() & 0xffff_ffff_0000_0000);
        let s = 1.0 + t * z;
        return t + a * (s + t * v);
    }
    if ix >= 0x3fe5_9428 {
        if hx < 0 {
            x = -x;
            y = -y;
        }
        let z = PIO4 - x;
        let w = PIO4_LO - y;
        x = z + w;
        y = 0.0;
    }
    let z = x * x;
    let w = z * z;
    let mut r = T[1] + w * (T[3] + w * (T[5] + w * (T[7] + w * (T[9] + w * T[11]))));
    let v = z * (T[2] + w * (T[4] + w * (T[6] + w * (T[8] + w * (T[10] + w * T[12])))));
    let s = z * x;
    r = y + z * (s * (r + v) + y);
    r += T[0] * s;
    let w = x + r;
    if ix >= 0x3fe5_9428 {
        let v = f64::from(iy);
        return f64::from(1 - ((hx >> 30) & 2)) * (v - 2.0 * (x - (w * w / (w + v) - r)));
    }
    if iy == 1 {
        w
    } else {
        let z = f64::from_bits(w.to_bits() & 0xffff_ffff_0000_0000);
        let v = r - (z - x);
        let a = -1.0 / w;
        let t = f64::from_bits(a.to_bits() & 0xffff_ffff_0000_0000);
        let s = 1.0 + t * z;
        t + a * (s + t * v)
    }
}

// Return (n, y0, y1), where x - n*pi/2 = y0+y1. For large arguments only
// n modulo 8 is needed, avoiding conversion of a huge quotient to an integer.
#[allow(clippy::eq_op)] // fdlibm: x - x is NaN for an infinite or NaN input, verbatim.
fn rem_pio2(x: f64) -> (i32, f64, f64) {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return (0, x, 0.0);
    }
    if ix < 0x4002_d97c {
        if hx > 0 {
            let mut z = x - PIO2_1;
            let (y0, y1) = if ix != 0x3ff9_21fb {
                let y0 = z - PIO2_1T;
                (y0, (z - y0) - PIO2_1T)
            } else {
                z -= PIO2_2;
                let y0 = z - PIO2_2T;
                (y0, (z - y0) - PIO2_2T)
            };
            return (1, y0, y1);
        }
        let mut z = x + PIO2_1;
        let (y0, y1) = if ix != 0x3ff9_21fb {
            let y0 = z + PIO2_1T;
            (y0, (z - y0) + PIO2_1T)
        } else {
            z += PIO2_2;
            let y0 = z + PIO2_2T;
            (y0, (z - y0) + PIO2_2T)
        };
        return (-1, y0, y1);
    }
    if ix <= 0x4139_21fb {
        let mut t = set_high_word(x, ix);
        let n = (t * INVPIO2 + 0.5) as i32;
        let fn_ = f64::from(n);
        let mut r = t - fn_ * PIO2_1;
        let mut w = fn_ * PIO2_1T;
        let mut y0 = r - w;
        if n >= 32 || ix == NPIO2_HW[(n - 1) as usize] {
            let j = ix >> 20;
            let mut i = j - ((high_word(y0) >> 20) & 0x7ff);
            if i > 16 {
                t = r;
                w = fn_ * PIO2_2;
                r = t - w;
                w = fn_ * PIO2_2T - ((t - r) - w);
                y0 = r - w;
                i = j - ((high_word(y0) >> 20) & 0x7ff);
                if i > 49 {
                    t = r;
                    w = fn_ * PIO2_3;
                    r = t - w;
                    w = fn_ * PIO2_3T - ((t - r) - w);
                    y0 = r - w;
                }
            }
        }
        let y1 = (r - y0) - w;
        return if hx < 0 { (-n, -y0, -y1) } else { (n, y0, y1) };
    }
    if ix >= 0x7ff0_0000 {
        return (0, x - x, x - x);
    }

    let e0 = (ix >> 20) - 1046;
    let mut z = set_high_word(x, ix - (e0 << 20));
    let mut tx = [0.0; 3];
    for part in &mut tx[..2] {
        *part = f64::from(z as i32);
        z = (z - *part) * TWO24;
    }
    tx[2] = z;
    let mut nx = 3;
    while tx[nx - 1] == 0.0 {
        nx -= 1;
    }
    let (n, y0, y1) = rem_pio2_large(&tx[..nx], e0);
    if hx < 0 { (-n, -y0, -y1) } else { (n, y0, y1) }
}

// All scalbn calls in the large-reduction kernel scale normal, bounded
// chunks by normal powers of two, without overflow or subnormal results.
// Thus multiplying by an exactly constructed 2^n has scalbn's exact result;
// no general-purpose platform scalbn or floor implementation is needed.
fn scale_chunk(x: f64, n: i32) -> f64 {
    x * f64::from_bits(((n + 1023) as u64) << 52)
}

// k_rem_pio2.c, specialized to prec=2 (jk=jp=4). Retains the original
// recomputation, carry/complement, and two-double compression stages.
fn rem_pio2_large(x: &[f64], e0: i32) -> (i32, f64, f64) {
    const JK: usize = 4;
    let jx = x.len() - 1;
    let jv = ((e0 - 3) / 24).max(0) as usize;
    let mut q0 = e0 - 24 * (jv as i32 + 1);
    let mut f = [0.0; 20];
    let mut fq = [0.0; 20];
    let mut q = [0.0; 20];
    let mut iq = [0_i32; 20];

    for (i, value) in f[..=jx + JK].iter_mut().enumerate() {
        let j = jv as i32 - jx as i32 + i as i32;
        *value = if j < 0 {
            0.0
        } else {
            f64::from(TWO_OVER_PI[j as usize])
        };
    }
    for i in 0..=JK {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw += x[j] * f[jx + i - j];
        }
        q[i] = fw;
    }

    let mut jz = JK;
    let (n, ih, mut z) = loop {
        // Distill q into 24-bit integer digits, from the least significant.
        let mut z = q[jz];
        for (i, j) in (1..=jz).rev().enumerate() {
            let fw = f64::from((TWON24 * z) as i32);
            iq[i] = (z - TWO24 * fw) as i32;
            z = q[j - 1] + fw;
        }
        z = scale_chunk(z, q0);
        // z is nonnegative and bounded here; truncation is exactly floor.
        z -= 8.0 * ((z * 0.125) as u64) as f64;
        let mut n = z as i32;
        z -= f64::from(n);
        let mut ih = 0;
        if q0 > 0 {
            let i = iq[jz - 1] >> (24 - q0);
            n += i;
            iq[jz - 1] -= i << (24 - q0);
            ih = iq[jz - 1] >> (23 - q0);
        } else if q0 == 0 {
            ih = iq[jz - 1] >> 23;
        } else if z >= 0.5 {
            ih = 2;
        }
        if ih > 0 {
            n += 1;
            let mut carry = false;
            for digit in &mut iq[..jz] {
                let j = *digit;
                if !carry {
                    if j != 0 {
                        carry = true;
                        *digit = 0x100_0000 - j;
                    }
                } else {
                    *digit = 0xff_ffff - j;
                }
            }
            match q0 {
                1 => iq[jz - 1] &= 0x7f_ffff,
                2 => iq[jz - 1] &= 0x3f_ffff,
                _ => {}
            }
            if ih == 2 {
                z = 1.0 - z;
                if carry {
                    z -= scale_chunk(1.0, q0);
                }
            }
        }
        if z == 0.0 {
            let mut j = 0;
            for &digit in &iq[JK..jz] {
                j |= digit;
            }
            if j == 0 {
                let mut k = 1;
                while iq[JK - k] == 0 {
                    k += 1;
                }
                for i in jz + 1..=jz + k {
                    f[jx + i] = f64::from(TWO_OVER_PI[jv + i]);
                    let mut fw = 0.0;
                    for j in 0..=jx {
                        fw += x[j] * f[jx + i - j];
                    }
                    q[i] = fw;
                }
                jz += k;
                continue;
            }
        }
        break (n, ih, z);
    };

    if z == 0.0 {
        jz -= 1;
        q0 -= 24;
        while iq[jz] == 0 {
            jz -= 1;
            q0 -= 24;
        }
    } else {
        z = scale_chunk(z, -q0);
        if z >= TWO24 {
            let fw = f64::from((TWON24 * z) as i32);
            iq[jz] = (z - TWO24 * fw) as i32;
            jz += 1;
            q0 += 24;
            iq[jz] = fw as i32;
        } else {
            iq[jz] = z as i32;
        }
    }
    let mut fw = scale_chunk(1.0, q0);
    for i in (0..=jz).rev() {
        q[i] = fw * f64::from(iq[i]);
        fw *= TWON24;
    }
    for i in (0..=jz).rev() {
        let mut fw = 0.0;
        for k in 0..=JK.min(jz - i) {
            fw += PIO2_CHUNKS[k] * q[i + k];
        }
        fq[jz - i] = fw;
    }
    let mut fw = 0.0;
    for i in (0..=jz).rev() {
        fw += fq[i];
    }
    let y0 = if ih == 0 { fw } else { -fw };
    fw = fq[0] - fw;
    for &part in &fq[1..=jz] {
        fw += part;
    }
    let y1 = if ih == 0 { fw } else { -fw };
    (n & 7, y0, y1)
}
