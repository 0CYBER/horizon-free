#![cfg(test)]
//! `ClientNarratorClientHost::narrate` (arismoko/horizon#4532): `Now` interrupts and `Queued`
//! does not, through the game's `GameNarrator`; an empty line, an option that
//! does not narrate system messages and missing speech are refused by name,
//! with nothing said or logged.
use super::client_narrator::say_for_mod;
use horizon_narrator::{GameNarrator, Sink, Status};
use mod_api::{NarrateError, Narration};
use std::sync::{Arc, Mutex};

/// What reached the speech: each line and whether it interrupted.
#[derive(Clone, Default)]
struct Recorder {
    said: Arc<Mutex<Vec<(String, bool)>>>,
    inactive: bool,
}
impl Sink for Recorder {
    fn say(&mut self, message: &str, interrupt: bool, _volume: f32) {
        self.said.lock().unwrap().push((message.into(), interrupt));
    }
    fn clear(&mut self) {}
    fn active(&mut self) -> bool {
        !self.inactive
    }
}

fn narrator(status: Status, recorder: &Recorder) -> GameNarrator {
    let mut narrator = GameNarrator::with_sink(Box::new(recorder.clone()));
    narrator.set_status(status);
    narrator
}

#[test]
fn now_interrupts_and_queued_does_not_for_system_narration() {
    for status in [Status::All, Status::System] {
        let recorder = Recorder::default();
        let mut narrator = narrator(status, &recorder);
        let now = say_for_mod(&mut narrator, "Oak Log", Narration::Now);
        let queued = say_for_mod(&mut narrator, "Stone", Narration::Queued);
        assert_eq!((now, queued), (Ok(()), Ok(())));
        assert_eq!(
            *recorder.said.lock().unwrap(),
            [("Oak Log".to_owned(), true), ("Stone".to_owned(), false)]
        );
        assert_eq!(narrator.recent(), ["Oak Log", "Stone"]);
    }
}

#[test]
fn a_long_line_is_said_whole_with_no_cap_of_its_own() {
    let recorder = Recorder::default();
    let mut narrator = narrator(Status::All, &recorder);
    let line = "Oak Log ".repeat(1000);
    let said = say_for_mod(&mut narrator, &line, Narration::Now);
    assert_eq!(said, Ok(()));
    assert_eq!(*recorder.said.lock().unwrap(), [(line, true)]);
}

#[test]
fn each_refusal_is_named_and_says_and_logs_nothing() {
    let cases = [
        (Status::All, false, "", NarrateError::Empty),
        (Status::Off, false, "Oak Log", NarrateError::Off),
        (Status::Chat, false, "Oak Log", NarrateError::Off),
        (Status::All, true, "Oak Log", NarrateError::Unavailable),
    ];
    for (status, inactive, line, refusal) in cases {
        for when in [Narration::Now, Narration::Queued] {
            let recorder = Recorder {
                said: Arc::new(Mutex::new(Vec::new())),
                inactive,
            };
            let mut narrator = narrator(status, &recorder);
            let answer = say_for_mod(&mut narrator, line, when);
            assert_eq!(answer, Err(refusal.clone()));
            assert!(recorder.said.lock().unwrap().is_empty());
            assert!(narrator.recent().is_empty());
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_server_threads_game_has_no_narrator_to_narrate_for() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let host: &mut dyn mod_api::ClientHost = game;
        assert_eq!(
            host.narrate(&mod_api::Text::literal("Oak Log"), Narration::Now),
            Err(NarrateError::Unavailable)
        );
    });
}
