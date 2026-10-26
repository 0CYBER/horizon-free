//! `DebugScreenOverlay`: the F3 text. The entries of the default profile
//! (`DebugScreenEntries.PROFILES`) report priority lines, regular lines and
//! groups; the overlay splits them into a left and a right column and draws
//! them last, over any screen, at the debug GUI scale.
use crate::{
    font::Font,
    hud_layout::{GuiDraw, PoseOp, fill},
    text::Text,
};

/// `extractLines`'s background: ARGB 0x90505050.
const BACKGROUND: u32 = 0x9050_5050;
/// `extractLines`'s text colour: ARGB 0xFFE0E0E0.
const TEXT: u32 = 0xFFE0_E0E0;
/// `Font.lineHeight`.
const LINE_HEIGHT: i32 = 9;

/// What the enabled entries reported (`DebugScreenDisplayer`).
#[derive(Default, Debug)]
pub struct Displayer {
    left: Vec<String>,
    right: Vec<String>,
    lines: Vec<String>,
    groups: Vec<(&'static str, Vec<String>)>,
}

impl Displayer {
    /// `addPriorityLine`: to the shorter column, the left one first.
    pub fn priority(&mut self, line: impl Into<String>) {
        if self.left.len() > self.right.len() {
            self.right.push(line.into());
        } else {
            self.left.push(line.into());
        }
    }
    /// `addLine`: split between the columns after the priority lines.
    pub fn line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
    /// `addToGroup`: lines kept together, groups in the order first added.
    pub fn group(&mut self, group: &'static str, lines: impl IntoIterator<Item = String>) {
        let index = match self.groups.iter().position(|(g, _)| *g == group) {
            Some(i) => i,
            None => {
                self.groups.push((group, Vec::new()));
                self.groups.len() - 1
            }
        };
        self.groups[index].1.extend(lines);
    }
    /// `extractRenderState`: the left and right columns, then `footer` (the
    /// chart keys, shown while the overlay is open) under the left one.
    pub fn columns(self, footer: &[String]) -> (Vec<String>, Vec<String>) {
        let (mut left, mut right) = (self.left, self.right);
        if !left.is_empty() {
            left.push(String::new());
        }
        if !right.is_empty() {
            right.push(String::new());
        }
        if !self.lines.is_empty() {
            let mid = self.lines.len().div_ceil(2);
            left.extend_from_slice(&self.lines[..mid]);
            right.extend_from_slice(&self.lines[mid..]);
            left.push(String::new());
            if mid < self.lines.len() {
                right.push(String::new());
            }
        }
        let mid = self.groups.len().div_ceil(2);
        for (i, (_, lines)) in self.groups.into_iter().enumerate() {
            if lines.is_empty() {
                continue;
            }
            let column = if i < mid { &mut left } else { &mut right };
            column.extend(lines);
            column.push(String::new());
        }
        if !footer.is_empty() {
            left.push(String::new());
            left.extend_from_slice(footer);
        }
        (left, right)
    }
}

/// Horizon's own lines: the shader pack's GPU time, then its costliest passes (`passes`: each
/// pass's median milliseconds, in the order they run).
pub fn shader_pass_lines(passes: &[(String, f32)]) -> Vec<String> {
    const SHOWN: usize = 8;
    if passes.is_empty() {
        return Vec::new();
    }
    let total: f32 = passes.iter().map(|p| p.1).sum();
    let mut lines = vec![format!(
        "Shader pack GPU: {total:.2} ms in {} passes",
        passes.len()
    )];
    let mut costliest: Vec<&(String, f32)> = passes.iter().collect();
    costliest.sort_by(|a, b| b.1.total_cmp(&a.1));
    lines.extend(
        costliest
            .iter()
            .take(SHOWN)
            .map(|(name, ms)| format!("{name}: {ms:.2} ms")),
    );
    if costliest.len() > SHOWN {
        let rest: f32 = costliest[SHOWN..].iter().map(|p| p.1).sum();
        lines.push(format!("{} more: {rest:.2} ms", costliest.len() - SHOWN));
    }
    lines
}

/// `Mth.wrapDegrees`.
fn wrap_degrees(angle: f32) -> f32 {
    let wrapped = angle % 360.0;
    if wrapped >= 180.0 {
        wrapped - 360.0
    } else if wrapped < -180.0 {
        wrapped + 360.0
    } else {
        wrapped
    }
}

/// `DebugEntryPosition` and `DebugEntrySectionPosition` for a camera at
/// `pos` looking `yaw`/`pitch` in `dimension` with `forced` force-loaded chunks.
pub fn position(
    pos: [f64; 3],
    yaw: f32,
    pitch: f32,
    dimension: &str,
    forced: usize,
) -> Vec<String> {
    let block = pos.map(|v| v.floor() as i32);
    let (cx, sy, cz) = (block[0] >> 4, block[1] >> 4, block[2] >> 4);
    // Direction.fromYRot: floor(yRot / 90 + 0.5) & 3 as a 2D data value.
    let (direction, towards) = match ((f64::from(yaw) / 90.0 + 0.5).floor() as i32) & 3 {
        0 => ("south", "Towards positive Z"),
        1 => ("west", "Towards negative X"),
        2 => ("north", "Towards negative Z"),
        _ => ("east", "Towards positive X"),
    };
    vec![
        format!("XYZ: {:.3} / {:.5} / {:.3}", pos[0], pos[1], pos[2]),
        format!("Block: {} {} {}", block[0], block[1], block[2]),
        format!(
            "Chunk: {cx} {sy} {cz} [{} {} in r.{}.{}.mca]",
            cx & 31,
            cz & 31,
            cx >> 5,
            cz >> 5
        ),
        format!(
            "Facing: {direction} ({towards}) ({:.1} / {:.1})",
            wrap_degrees(yaw),
            wrap_degrees(pitch)
        ),
        format!("{dimension} FC: {forced}"),
        format!(
            "Section-relative: {:02} {:02} {:02}",
            block[0] & 15,
            block[1] & 15,
            block[2] & 15
        ),
    ]
}

/// `VideoMode.refreshRateLabel`: whole rates without decimals.
pub fn refresh_rate_label(hz: f64) -> String {
    if hz == hz.round() {
        format!("{}", hz as i64)
    } else {
        format!("{hz:.2}")
    }
}

/// `DeviceInfo.vendorName` for a PCI vendor id.
pub fn vendor_name(id: u32) -> String {
    match id {
        0x10de => "NVIDIA Corporation".into(),
        0x1002 | 0x1022 => "AMD".into(),
        0x8086 => "Intel".into(),
        0x106b => "Apple".into(),
        0x5143 => "Qualcomm".into(),
        0x13b5 => "ARM".into(),
        0x10005 => "Mesa".into(),
        0 => "unknown".into(),
        other => format!("{other:#06x}"),
    }
}

/// The overlay's scale against the GUI scale and the scaled screen size it
/// lays out in. `debugGuiScale`: -1 keeps the GUI scale, 0 (the default) is
/// half the largest automatic scale, n is `calculateScale(n)`; only a scale
/// below the GUI scale applies.
pub fn scale(
    framebuffer: [i32; 2],
    gui_scale: i32,
    option: i32,
    unicode: bool,
    gui_size: [i32; 2],
) -> (f32, [i32; 2]) {
    let scale = match option {
        -1 => gui_scale,
        0 => crate::window::calculate_scale(framebuffer, 0, unicode) / 2,
        n => crate::window::calculate_scale(framebuffer, n, unicode),
    };
    if scale < gui_scale && scale > 0 {
        (
            scale as f32 / gui_scale as f32,
            [framebuffer[0] / scale, framebuffer[1] / scale],
        )
    } else {
        (1.0, gui_size)
    }
}

/// `extractLines` for both columns under the overlay's pose: each line's
/// background first, then the text, right-aligned in the right column.
pub fn draw(font: &Font, columns: [&[String]; 2], scale: f32, width: i32) -> Vec<GuiDraw> {
    let mut out = vec![GuiDraw::Push(vec![PoseOp::Scale(scale, scale)])];
    for (lines, align_left) in [(columns[0], true), (columns[1], false)] {
        let placed: Vec<(&String, i32, i32, i32)> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.is_empty())
            .map(|(i, l)| {
                let w = font.width(l) as i32;
                let left = if align_left { 2 } else { width - 2 - w };
                (l, left, 2 + LINE_HEIGHT * i as i32, w)
            })
            .collect();
        for (_, left, top, w) in &placed {
            out.push(fill(
                left - 1,
                top - 1,
                left + w + 1,
                top + LINE_HEIGHT - 1,
                BACKGROUND,
            ));
        }
        for (line, left, top, _) in placed {
            out.push(GuiDraw::Text {
                text: Text::literal(line.clone()),
                x: left,
                y: top,
                color: TEXT,
                shadow: false,
            });
        }
    }
    out.push(GuiDraw::Pop);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn columns_follow_the_vanilla_split() {
        // The default profile's entries, sorted by identifier as
        // DebugScreenEntryList.rebuildCurrentList does: fps, game_version,
        // memory, player_position, player_section_position,
        // simple_performance_impactors, system_specs, tps.
        let mut d = Displayer::default();
        d.priority("112 fps T: 120 (fifo) @165Hz");
        d.priority("Minecraft 26.3 (26.3/vanilla)");
        d.group("memory", lines(&["Mem: 25% 1046/4096MiB"]));
        d.group("position", lines(&["XYZ: 1 / 2 / 3", "Block: 1 2 3"]));
        d.group("position", lines(&["Section-relative: 01 02 03"]));
        d.line("fancy-clouds B: 2");
        d.line("Filtering: RGSS");
        d.line("Terrain Rendering: multidrawindirect");
        d.group(
            "system",
            lines(&["CPU: 12x Ryzen", "Display: 854x709 (NVIDIA)"]),
        );
        d.line("Integrated server @ 6.6/50.0 ms, 28 tx, 492 rx");
        let footer = lines(&["Debug charts: ...", "To edit: press [F3+F6]"]);
        let (left, right) = d.columns(&footer);
        // As the issue's vanilla screenshot (#376) shows them.
        assert_eq!(
            left,
            lines(&[
                "112 fps T: 120 (fifo) @165Hz",
                "",
                "fancy-clouds B: 2",
                "Filtering: RGSS",
                "",
                "Mem: 25% 1046/4096MiB",
                "",
                "XYZ: 1 / 2 / 3",
                "Block: 1 2 3",
                "Section-relative: 01 02 03",
                "",
                "",
                "Debug charts: ...",
                "To edit: press [F3+F6]",
            ])
        );
        assert_eq!(
            right,
            lines(&[
                "Minecraft 26.3 (26.3/vanilla)",
                "",
                "Terrain Rendering: multidrawindirect",
                "Integrated server @ 6.6/50.0 ms, 28 tx, 492 rx",
                "",
                "CPU: 12x Ryzen",
                "Display: 854x709 (NVIDIA)",
                "",
            ])
        );
    }

    #[test]
    fn position_lines_match_vanilla() {
        // The issue's vanilla screenshot (#376).
        assert_eq!(
            position([108.5, 104.0, -51.5], -1.0, 7.1, "minecraft:overworld", 0),
            lines(&[
                "XYZ: 108.500 / 104.00000 / -51.500",
                "Block: 108 104 -52",
                "Chunk: 6 6 -4 [6 28 in r.0.-1.mca]",
                "Facing: south (Towards positive Z) (-1.0 / 7.1)",
                "minecraft:overworld FC: 0",
                "Section-relative: 12 08 12",
            ])
        );
        assert_eq!(
            position([0.0, 64.0, 0.0], 450.0, 0.0, "minecraft:the_nether", 2)[3],
            "Facing: west (Towards negative X) (90.0 / 0.0)"
        );
        assert_eq!(refresh_rate_label(165.0), "165");
        assert_eq!(refresh_rate_label(59.94), "59.94");
    }

    #[test]
    fn debug_scale_is_half_the_automatic_scale() {
        // The issue's window: 854x709 at GUI scale 2 draws F3 at scale 1.
        assert_eq!(
            scale([854, 709], 2, 0, false, [427, 355]),
            (0.5, [854, 709])
        );
        // 1920x1080 at GUI scale 4: auto is 4, so F3 is at 2.
        assert_eq!(
            scale([1920, 1080], 4, 0, false, [480, 270]),
            (0.5, [960, 540])
        );
        // A GUI scale already at or under the debug scale is kept.
        assert_eq!(
            scale([1920, 1080], 2, 0, false, [960, 540]),
            (1.0, [960, 540])
        );
        assert_eq!(
            scale([1920, 1080], 3, -1, false, [640, 360]),
            (1.0, [640, 360])
        );
    }

    #[test]
    fn shader_pass_lines_list_the_costliest_passes_first() {
        assert!(shader_pass_lines(&[]).is_empty());
        let passes: Vec<(String, f32)> = (0..10).map(|i| (format!("p{i}"), i as f32)).collect();
        let lines = shader_pass_lines(&passes);
        assert_eq!(lines[0], "Shader pack GPU: 45.00 ms in 10 passes");
        assert_eq!(lines[1], "p9: 9.00 ms");
        assert_eq!(lines[8], "p2: 2.00 ms");
        assert_eq!(lines[9], "2 more: 1.00 ms");
    }
}
