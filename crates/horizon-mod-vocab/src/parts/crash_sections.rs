// exports: none (reached inside the crate only)
//! Crash report lines a mod adds (NeoForge's `CrashReportCallables`), computed when a report is
//! written rather than when they are registered.
use super::*;
use crate::wire::{Reader, Wire, WireError, Writer};

crate::wire::wire_handlers::handler!(dyn CrashSection, "CrashSection");

/// A crash report line a mod computes when the report is written: NeoForge's
/// `ISystemReportExtender`, `ICrashReportHeader`, or a `registerCrashCallable` supplier with its
/// active check. The report asks it once, on a thread of its own, while the game may have stopped
/// mid-tick: read the mod's own state, and don't wait on anything the game holds.
pub trait CrashSection: Send + Sync + 'static {
    /// The line's text, or `None` to leave it out of this report. The report keeps it on one
    /// line (control characters become spaces) and cuts it to 4096 bytes.
    fn value(&self) -> Option<String>;
}

/// The registration side of [`CrashSection`]: what [`Registrar`] adds them with.
pub trait CrashSectionsRegistrar {
    /// `registerCrashCallable(label, supplier)`: a line of every crash report's system details,
    /// `<mod id>: <label>: <value>`, after Horizon's own lines, in mod load order. A section that
    /// panics, traps or doesn't answer within the report's wait reads `~~ERROR~~`, and the report
    /// is still written. The lines of the mod the crash is attributed to are not asked.
    fn crash_section(&mut self, label: &str, section: Arc<dyn CrashSection>);
    /// `registerHeader`: a line under the report's title, `<mod id>: <value>`, asked as a
    /// [`crash_section`](Self::crash_section) is.
    fn crash_header(&mut self, header: Arc<dyn CrashSection>);
}

#[derive(Clone)]
pub(crate) struct CrashSectionPart {
    pub(crate) mod_id: &'static str,
    pub(crate) mod_index: usize,
    /// The detail's label; `None` for a header line.
    pub(crate) label: Option<Box<str>>,
    pub(crate) section: Arc<dyn crate::CrashSection>,
}

/// The crash report lines the enabled mods registered, in registration order: the mod's
/// `[mod] id`, the label (`None` for a header line) and what computes the line.
pub fn mod_crash_lines() -> Vec<(&'static str, Option<Box<str>>, Arc<dyn CrashSection>)> {
    installed().map_or_else(Vec::new, |installed| {
        installed
            .keys
            .crash_sections
            .iter()
            .filter(|line| crate::mod_index_on(line.mod_index))
            .map(|line| (line.mod_id, line.label.clone(), line.section.clone()))
            .collect()
    })
}
