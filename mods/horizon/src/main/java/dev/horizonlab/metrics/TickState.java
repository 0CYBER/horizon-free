package dev.horizonlab.metrics;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.Map;

/** Scalar-only observations of the tick manager after each measured server invocation. */
public final class TickState {
    private static final int FROZEN = 1;
    private static final int STEPPING = 2;
    private static final int SPRINTING = 4;
    private static final int RUNS_GAME_ELEMENTS = 8;
    private long count;
    private float firstRate;
    private float lastRate;
    private int firstFlags;
    private int lastFlags;
    private int changedFlags;
    private boolean rateChanged;
    private long frozen;
    private long stepping;
    private long sprinting;
    private long suppressed;
    private long nonstandardRate;
    private long invalid;

    public void observe(float rate, boolean isFrozen, boolean isStepping, boolean isSprinting, boolean runsGameElements) {
        if (!Float.isFinite(rate) || rate < 1) { invalid++; return; }
        int flags = (isFrozen ? FROZEN : 0) | (isStepping ? STEPPING : 0)
            | (isSprinting ? SPRINTING : 0) | (runsGameElements ? RUNS_GAME_ELEMENTS : 0);
        if (count == 0) {
            firstRate = rate;
            firstFlags = flags;
        } else {
            rateChanged |= rate != lastRate;
            changedFlags |= flags ^ lastFlags;
        }
        lastRate = rate;
        lastFlags = flags;
        count++;
        if (isFrozen) frozen++;
        if (isStepping) stepping++;
        if (isSprinting) sprinting++;
        if (!runsGameElements) suppressed++;
        if (rate != 20) nonstandardRate++;
    }

    public Map<String, Object> metadata() {
        Map<String, Object> result = new LinkedHashMap<>();
        result.put("tick_state_capture_status", invalid > 0 ? "invalid_samples" : count == 0 ? "unavailable" : "sampled_server_ticks");
        result.put("tick_state_samples", count);
        result.put("tick_state_invalid_samples", invalid);
        if (count == 0) return result;
        result.put("tick_state_first", state(firstRate, firstFlags));
        result.put("tick_state_last", state(lastRate, lastFlags));
        var changed = new ArrayList<String>();
        if (rateChanged) changed.add("configured_tick_rate");
        if ((changedFlags & FROZEN) != 0) changed.add("frozen");
        if ((changedFlags & STEPPING) != 0) changed.add("stepping");
        if ((changedFlags & SPRINTING) != 0) changed.add("sprinting");
        if ((changedFlags & RUNS_GAME_ELEMENTS) != 0) changed.add("runs_game_elements");
        result.put("tick_state_changed", changed);
        result.put("tick_state_frozen_samples", frozen);
        result.put("tick_state_stepping_samples", stepping);
        result.put("tick_state_sprinting_samples", sprinting);
        result.put("tick_state_suppressed_samples", suppressed);
        result.put("tick_state_nonstandard_rate_samples", nonstandardRate);
        return result;
    }

    private static Map<String, Object> state(float rate, int flags) {
        return Map.of("configured_tick_rate", rate, "frozen", (flags & FROZEN) != 0,
            "stepping", (flags & STEPPING) != 0, "sprinting", (flags & SPRINTING) != 0,
            "runs_game_elements", (flags & RUNS_GAME_ELEMENTS) != 0);
    }
}
