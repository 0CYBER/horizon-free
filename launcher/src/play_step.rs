//! The step Play is on before the game's process exists, as the start's threads say it. Checking
//! the assets, updating registry mods, reading the sign-in, a pack's carried mods and the Java
//! jars can take minutes on a modded instance, and the window said only "Starting".
use horizon_launcher::backend::{self, Msg};
use std::sync::mpsc::Sender;

/// Says the start's step: a line on stderr, so a terminal run shows where Play stops (#12179),
/// and `shown` to the window's label and Console.
pub(crate) fn say(tx: &Sender<Msg>, stderr: &str, shown: String) {
    backend::play_step(stderr);
    let _ = tx.send(Msg::PlayStep(shown));
}

/// The step the start in progress said last.
#[derive(Default)]
pub(crate) struct PlayStep(Option<String>);

impl PlayStep {
    /// A step a start said, while one is `starting`: the line for the Console. One arriving
    /// after its start ended is dropped.
    pub(crate) fn said(&mut self, starting: bool, step: String) -> Option<String> {
        starting.then(|| {
            self.0 = Some(step.clone());
            step
        })
    }

    /// After the window's messages: with no start in progress (the game started, or the start
    /// failed or was refused), no step.
    pub(crate) fn settle(&mut self, starting: bool) {
        if !starting {
            self.0 = None;
        }
    }

    pub(crate) fn now(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The steps a start's thread says reach the window in their order, each shown as it arrives
    /// and kept for the Console, and the step clears when the start ends; one said after the end
    /// is dropped.
    #[test]
    fn steps_reach_the_window_in_order_and_clear_when_the_start_ends() {
        let (tx, rx) = std::sync::mpsc::channel();
        let steps = [
            "Checking the game's assets…",
            "Reading the sign-in…",
            "Preparing Java jars…",
        ];
        std::thread::spawn(move || {
            for step in steps {
                say(&tx, "a step", step.to_owned());
            }
            let _ = tx.send(Msg::JarClient {
                run: None,
                prepared: Box::new(Err("refused".into())),
            });
            say(&tx, "late", "Too late…".to_owned());
        })
        .join()
        .unwrap();

        let mut step = PlayStep::default();
        let (mut starting, mut shown, mut console) = (true, Vec::new(), Vec::new());
        for message in rx.try_iter() {
            match message {
                Msg::PlayStep(said) => console.extend(step.said(starting, said)),
                Msg::JarClient { .. } => starting = false,
                _ => panic!("another message"),
            }
            step.settle(starting);
            shown.push(step.now().map(str::to_owned));
        }
        assert_eq!(console, steps);
        let some = |s: &str| Some(s.to_owned());
        assert_eq!(
            shown,
            [some(steps[0]), some(steps[1]), some(steps[2]), None, None]
        );
    }
}
