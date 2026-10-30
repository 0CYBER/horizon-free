//! `net.minecraft.util.worldupdate.UpgradeProgress`: what a world upgrade has
//! done so far, read by `FileFixerProgressScreen` while another thread works.
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, Ordering};

/// `UpgradeProgress.FileFixStats`.
#[derive(Debug, Default)]
pub struct FileFixStats {
    finished_operations: AtomicI32,
    total_operations: AtomicI32,
}

impl FileFixStats {
    pub fn finished_operations(&self) -> i32 {
        self.finished_operations.load(Ordering::Relaxed)
    }
    pub fn total_operations(&self) -> i32 {
        self.total_operations.load(Ordering::Relaxed)
    }
    pub fn reset(&self) {
        self.finished_operations.store(0, Ordering::Relaxed);
        self.total_operations.store(0, Ordering::Relaxed);
    }
    /// `getProgress`: finished over total, 0 without operations, in 0..=1.
    pub fn progress(&self) -> f32 {
        if self.total_operations() == 0 {
            0.0
        } else {
            (self.finished_operations() as f32 / self.total_operations() as f32).clamp(0.0, 1.0)
        }
    }
}

/// `UpgradeProgress.Type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeType {
    Files,
    LegacyStructures,
    Regions,
}

impl UpgradeType {
    /// `getSerializedName`.
    pub fn serialized_name(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::LegacyStructures => "legacy_structures",
            Self::Regions => "regions",
        }
    }
    /// The key of `label()`: `upgradeWorld.progress.type.` + the serialized name.
    pub fn label_key(self) -> String {
        format!("upgradeWorld.progress.type.{}", self.serialized_name())
    }
    fn code(self) -> u8 {
        match self {
            Self::Files => 1,
            Self::LegacyStructures => 2,
            Self::Regions => 3,
        }
    }
}

/// `UpgradeProgress`, as far as the file fixer's screen reads it.
#[derive(Debug, Default)]
pub struct UpgradeProgress {
    total: FileFixStats,
    of_type: FileFixStats,
    running_fixer: FileFixStats,
    canceled: AtomicBool,
    /// `UpgradeProgress.type`: 0 for none.
    ty: AtomicU8,
    /// The upgrade runs the file fixers (`requiresFileFixing`), which is what
    /// opens `FileFixerProgressScreen` (`WorldOpenFlows.upgradeAndOpenWorld`).
    file_fixing: AtomicBool,
}

impl UpgradeProgress {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn total_file_fix_stats(&self) -> &FileFixStats {
        &self.total
    }
    pub fn type_file_fix_stats(&self) -> &FileFixStats {
        &self.of_type
    }
    pub fn running_file_fixer_stats(&self) -> &FileFixStats {
        &self.running_fixer
    }
    pub fn add_total_file_fix_operations(&self, additional: i32) {
        self.total
            .total_operations
            .fetch_add(additional, Ordering::Relaxed);
        self.of_type
            .total_operations
            .fetch_add(additional, Ordering::Relaxed);
    }
    pub fn increment_finished_operations(&self) {
        self.increment_finished_operations_by(1);
    }
    pub fn increment_finished_operations_by(&self, count: i32) {
        self.total
            .finished_operations
            .fetch_add(count, Ordering::Relaxed);
        self.of_type
            .finished_operations
            .fetch_add(count, Ordering::Relaxed);
    }
    pub fn set_canceled(&self) {
        self.canceled.store(true, Ordering::Relaxed);
    }
    pub fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }
    /// `setType`: the type's own stats start over.
    pub fn set_type(&self, ty: UpgradeType) {
        self.ty.store(ty.code(), Ordering::Relaxed);
        self.of_type.reset();
    }
    pub fn upgrade_type(&self) -> Option<UpgradeType> {
        match self.ty.load(Ordering::Relaxed) {
            1 => Some(UpgradeType::Files),
            2 => Some(UpgradeType::LegacyStructures),
            3 => Some(UpgradeType::Regions),
            _ => None,
        }
    }
    pub fn set_applicable_fixer_amount(&self, amount: i32) {
        self.running_fixer
            .total_operations
            .store(amount, Ordering::Relaxed);
    }
    pub fn increment_running_file_fixer(&self) {
        self.running_fixer
            .finished_operations
            .fetch_add(1, Ordering::Relaxed);
    }
    pub fn set_file_fixing(&self) {
        self.file_fixing.store(true, Ordering::Relaxed);
    }
    pub fn is_file_fixing(&self) -> bool {
        self.file_fixing.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_follow_the_operations_and_the_type_starts_over() {
        let p = UpgradeProgress::new();
        assert_eq!(p.total_file_fix_stats().progress(), 0.0);
        p.set_type(UpgradeType::Files);
        p.add_total_file_fix_operations(4);
        p.increment_finished_operations();
        assert_eq!(p.total_file_fix_stats().progress(), 0.25);
        p.set_type(UpgradeType::Regions);
        assert_eq!(p.type_file_fix_stats().total_operations(), 0);
        assert_eq!(p.total_file_fix_stats().total_operations(), 4);
        p.increment_finished_operations_by(9);
        assert_eq!(p.type_file_fix_stats().progress(), 0.0);
        assert_eq!(p.total_file_fix_stats().progress(), 1.0);
        assert_eq!(
            p.upgrade_type().unwrap().label_key(),
            "upgradeWorld.progress.type.regions"
        );
    }
}
