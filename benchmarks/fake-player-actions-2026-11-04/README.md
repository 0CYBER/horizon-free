# Fake players acting each tick (`PlayerRef::perform_action`)

`game::native_server::fake_action_tests::fakes_acting_each_tick_cost`: 1, 64 and 256 fake players each acting once
a tick, 200 ticks, release build:
- `UseOn`: clicking the stone floor with an empty hand (the full admission and use path; vanilla passes);
- `Attack`: hitting a zombie beside them with a fist (its health is reset each tick, so it never dies);
- `BreakBlock`: breaking a stone block, put back before each break.

```sh
HORIZON_TEST_RESOURCES=.local/horizon-client/26.3 cargo test --release -p horizon-client --lib --bins --tests \
  fakes_acting_each_tick_cost -- --ignored --nocapture
```

AMD Ryzen 7 5700X, Linux, 2026-11-04. An action's cost, and a tick's at 256 fakes:

| Action | 1 fake | 64 fakes | 256 fakes | A tick at 256 |
|---|---|---|---|---|
| `UseOn` | 4.3 µs | 4.7 µs | 6.3 µs | 1.62 ms |
| `Attack` | 9.4 µs | 6.8 µs | 7.2 µs | 1.85 ms |
| `BreakBlock` | 4.8 µs | 5.1 µs | 6.2 µs | 1.59 ms |

256 fakes acting every tick take 3 to 4% of a 50 ms tick. Nothing runs when no mod calls `perform_action`.

Where the time goes, measured with temporary timers around the player's turn (`as_guest_at`): entering and
leaving the turn cost about 1 µs each, the same at 1 and at 256 fakes. What grows with the number of fakes is the
action itself: `UseOn`'s use took 1.9 µs with one fake and 3.3 µs with 256. Finding the fake in the guest list is not
what grows: `perform_action` finds it once and starts its turn at that place in the list (before this run it was
found three times, at 4.3, 4.6 and 6.0 µs an action). An `Attack` enters the turn three times, for the attack range,
the native mods' `attack_entity` and the blow, as the packet does.
