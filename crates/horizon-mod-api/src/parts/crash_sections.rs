//! Concrete crash report line authoring; the frozen list stays in the vocabulary.
use super::*;

impl CrashSectionsRegistrar for PartRegistrar {
    fn crash_section(&mut self, label: &str, section: Arc<dyn CrashSection>) {
        self.add_crash_line(Some(label.into()), section);
    }
    fn crash_header(&mut self, header: Arc<dyn CrashSection>) {
        self.add_crash_line(None, header);
    }
}

impl PartRegistrar {
    fn add_crash_line(&mut self, label: Option<Box<str>>, section: Arc<dyn CrashSection>) {
        self.parts.crash_sections.push(CrashSectionPart {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            label,
            section,
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::{CrashSection, Hooks, ModEntry, Registrar, RegistrationContext};
    use std::sync::Arc;

    struct Line(&'static str);
    impl CrashSection for Line {
        fn value(&self) -> Option<String> {
            Some(self.0.into())
        }
    }

    struct Lines;
    impl Hooks for Lines {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.crash_section("caches", Arc::new(Line("3 caches")));
            r.crash_header(Arc::new(Line("modded")));
        }
    }

    struct Refused;
    impl Hooks for Refused {
        fn subscribed(&self) -> &'static [Option<crate::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {
            r.crash_section("never", Arc::new(Line("never")));
            cx.refuse("bootstrap failed");
        }
    }

    #[test]
    fn lines_keep_their_mod_and_order_and_a_refused_mod_leaves_none() {
        let mods = [
            ModEntry::new("lines", Box::new(Lines)),
            ModEntry::new("refused", Box::new(Refused)),
        ];
        let registrar = super::super::register_all(&mods, |_| false);
        let lines: Vec<_> = registrar
            .parts
            .crash_sections
            .iter()
            .map(|line| (line.mod_id, line.label.clone(), line.section.value()))
            .collect();
        assert_eq!(
            lines,
            [
                (
                    "lines",
                    Some(Box::from("caches")),
                    Some(String::from("3 caches"))
                ),
                ("lines", None, Some(String::from("modded"))),
            ]
        );
    }
}
