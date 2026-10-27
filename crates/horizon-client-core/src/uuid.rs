//! UUIDs as vanilla's four-int form, to and from their text form.

pub fn uuid_text(u: [i32; 4]) -> String {
    let hex: String = u.iter().map(|v| format!("{:08x}", *v as u32)).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}
pub fn uuid_from_text(s: &str) -> Option<[i32; 4]> {
    let hex: String = s.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0i32; 4];
    for (i, v) in out.iter_mut().enumerate() {
        *v = u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok()? as i32;
    }
    Some(out)
}
