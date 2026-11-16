# Measuring a mod's calls

The server's `/modprofile` command measures how long native mods spend in their hooks and Parts. It needs
permission level 3 and is off until you start it.

1. `/modprofile start` starts a measurement. Starting twice reports an error.
2. Play through what you want to measure.
3. `/modprofile stop` ends it and saves `profiles/modprofile-<timestamp>-<counter>.json` in the game folder.
   Stopping with no measurement running reports an error.
4. `/modprofile report` prints the last finished summary, without saving a file or stopping a running measurement.

Replies go only to the player who ran the command. The same controls work through
`horizon-ctl command modprofile start` (and `stop` or `report`).

## The profile file

Open the file in [Firefox Profiler](https://profiler.firefox.com) and look at its Marker Table. Horizon never uploads
it.

- The file is written atomically and is at most 8 MiB. If saving fails, the command says so and the summary stays
  available to `report`.
- Each marker names a mod, a hook or Part and holds call counts, exact timings in nanoseconds and log2 histograms. It
  never holds arguments, player data or world data.
- Timings are inclusive (an outer call includes the calls inside it) and add up over the whole process: they are
  totals, not a trace of each call or a sampling profile.
- Only native mods' calls are measured.
