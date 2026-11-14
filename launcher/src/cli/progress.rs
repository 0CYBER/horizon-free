//! Download progress presentation for terminal and redirected CLI output.
use horizon_launcher::native_mods;
use std::io::{IsTerminal, Write};

/// Download progress on one terminal line, only when stderr is a terminal and the percentage moves.
pub(super) fn progress(label: String) -> impl Fn(f32) {
    let last = std::cell::Cell::new(-1);
    let tty = std::io::stderr().is_terminal();
    move |p| {
        let pct = (p * 100.) as i32;
        if !tty || pct == last.replace(pct) {
            return;
        }
        eprint!("\r{label} {pct:>3}%");
        let _ = std::io::stderr().flush();
        if p >= 1.0 {
            eprintln!();
        }
    }
}

/// Keep detailed terminal updates, but only the first and completed download updates in logs.
pub(super) fn native_progress(tty: bool) -> impl Fn(native_mods::Step) -> Option<String> {
    use native_mods::Step;
    let previous = std::cell::RefCell::new(None);
    move |step| {
        let show = tty
            || match (&step, previous.borrow().as_ref()) {
                (Step::Rust(done), Some(Step::Rust(last))) => *done >= 1.0 && *last < 1.0,
                (
                    Step::Tools { build, done, total },
                    Some(Step::Tools {
                        build: last_build,
                        done: last_done,
                        total: last_total,
                    }),
                ) if build == last_build => {
                    *total > 0 && done >= total && (*last_total == 0 || last_done < last_total)
                }
                _ => true,
            };
        let label = show.then(|| step.label());
        *previous.borrow_mut() = Some(step);
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_progress_limits_redirected_rust_downloads_to_start_and_end() {
        let progress = native_progress(false);
        let fractions = [0.0, 0.01, 0.97, 0.971, 0.974, 0.98, 1.0, 1.0];
        let labels: Vec<_> = fractions
            .into_iter()
            .filter_map(|done| progress(native_mods::Step::Rust(done)))
            .collect();
        assert_eq!(
            labels,
            [
                native_mods::Step::Rust(0.0).label(),
                native_mods::Step::Rust(1.0).label(),
            ]
        );
    }

    #[test]
    fn native_progress_keeps_detailed_terminal_downloads() {
        let progress = native_progress(true);
        for done in [0.0, 0.97, 0.971, 0.974, 0.98, 1.0] {
            let step = native_mods::Step::Rust(done);
            assert_eq!(progress(step.clone()), Some(step.label()));
        }
        for done in [0, 1, 97, 100] {
            let step = native_mods::Step::Tools {
                build: "sdk".into(),
                done,
                total: 100,
            };
            assert_eq!(progress(step.clone()), Some(step.label()));
        }
    }

    #[test]
    fn native_progress_limits_each_redirected_sdk_download() {
        let progress = native_progress(false);
        let tools = |build: &str, done, total| native_mods::Step::Tools {
            build: build.into(),
            done,
            total,
        };
        for (done, shown) in [
            (0, true),
            (1, false),
            (97, false),
            (100, true),
            (100, false),
        ] {
            let step = tools("first", done, 100);
            assert_eq!(progress(step.clone()), shown.then(|| step.label()));
        }
        let next = tools("second", 20, 100);
        assert_eq!(progress(next.clone()), Some(next.label()));
        assert_eq!(progress(tools("second", 50, 100)), None);
        let complete = tools("second", 100, 100);
        assert_eq!(progress(complete.clone()), Some(complete.label()));
        let unknown = tools("third", 0, 0);
        assert_eq!(progress(unknown.clone()), Some(unknown.label()));
        assert_eq!(progress(tools("third", 0, 0)), None);
        let complete = tools("third", 100, 100);
        assert_eq!(progress(complete.clone()), Some(complete.label()));
    }

    #[test]
    fn native_progress_keeps_phase_order_and_restarts_download_reporting() {
        use native_mods::Step;
        let progress = native_progress(false);
        let first = Step::Rust(0.97);
        assert_eq!(progress(first.clone()), Some(first.label()));
        assert_eq!(progress(Step::Rust(0.98)), None);
        for step in [
            Step::PrebuiltMods,
            Step::Compile {
                done: 1,
                total: Some(2),
                name: "first".into(),
                profile: Default::default(),
            },
            Step::Compile {
                done: 2,
                total: Some(2),
                name: "second".into(),
                profile: Default::default(),
            },
            Step::Install,
            Step::Rust(0.0),
            Step::Rust(1.0),
        ] {
            assert_eq!(progress(step.clone()), Some(step.label()));
        }
    }
}
