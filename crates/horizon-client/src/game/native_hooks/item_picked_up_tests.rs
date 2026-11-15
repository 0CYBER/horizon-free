#![cfg(test)]
use crate::game::Game;
use mod_api::TriState;

const ME: [i32; 4] = [1, 2, 3, 4];
const OTHER: [i32; 4] = [5, 6, 7, 8];

/// `(delay after, taken)` for an item with `delay` and `target` touched by `ME`.
fn decide(mut delay: i64, target: Option<[i32; 4]>, answer: TriState) -> (i64, bool) {
    let taken = Game::pickup_decision(&mut delay, target, ME, answer);
    (delay, taken)
}

#[test]
fn no_opinion_keeps_vanilla_rule() {
    assert_eq!(decide(0, None, TriState::Default), (0, true));
    assert_eq!(decide(0, Some(ME), TriState::Default), (0, true));
    assert_eq!(decide(0, Some(OTHER), TriState::Default), (0, false));
    assert_eq!(decide(40, None, TriState::Default), (40, false));
    assert_eq!(decide(40, Some(ME), TriState::Default), (40, false));
}

#[test]
fn allow_overrides_delay_and_target() {
    assert_eq!(decide(40, None, TriState::True), (40, true));
    assert_eq!(decide(0, Some(OTHER), TriState::True), (0, true));
    assert_eq!(decide(32767, Some(OTHER), TriState::True), (32767, true));
}

#[test]
fn deny_leaves_the_item_for_at_least_ten_ticks() {
    assert_eq!(decide(0, None, TriState::False), (10, false));
    assert_eq!(decide(3, Some(ME), TriState::False), (10, false));
    assert_eq!(decide(40, None, TriState::False), (40, false));
}
