package dev.horizonlab.client;

import com.seibel.distanthorizons.api.DhApi;
import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfigValue;
import dev.horizonlab.HorizonLab;
import dev.horizonlab.budget.TerrainBudget;

/** Loaded only when DH is installed. Overrides are temporary, never written to its TOML. */
final class DistantTerrainBudget {
    private IDhApiConfigValue<Double> setting;
    private TerrainBudget policy;
    private Double owned;
    private boolean yielded;
    private long windowStart;
    private long totalNs;
    private int count;
    private int late;
    private int changes;

    void observe(long now, long frameNs, double targetMs) {
        if (yielded || DhApi.Delayed.configs == null) return;
        if (setting == null) {
            setting = DhApi.Delayed.configs.multiThreading().threadRuntimeRatio();
            if (!setting.getCanBeOverrodeByApi() || setting.getApiValue() != null) {
                yielded = true;
                HorizonLab.LOGGER.info("Adaptive terrain budget yielded to an existing DH override");
                return;
            }
            policy = new TerrainBudget(setting.getTrueValue());
        }
        if (!java.util.Objects.equals(setting.getApiValue(), owned)) {
            yielded = true;
            owned = null;
            HorizonLab.LOGGER.info("Adaptive terrain budget yielded after another DH override changed");
            return;
        }
        if (windowStart == 0) windowStart = now;
        if (frameNs > 0) {
            totalNs += frameNs;
            count++;
            if (frameNs / 1e6 > targetMs * 2) late++;
        }
        if (now - windowStart < 2_000_000_000L || count == 0) return;
        Runtime runtime = Runtime.getRuntime();
        double heap = (runtime.totalMemory() - runtime.freeMemory()) / (double) runtime.maxMemory();
        double next = policy.update(setting.getTrueValue(), totalNs / 1e6 / count, targetMs,
            late / (double) count, heap);
        if (owned == null || Math.abs(next - owned) > 0.0001) {
            if (setting.setValue(next, "Horizon adaptive budget")) {
                owned = next;
                changes++;
                HorizonLab.LOGGER.info("Adaptive terrain background runtime ratio: {}", next);
            }
        }
        resetWindow();
    }

    void suspend() {
        if (setting != null && owned != null && java.util.Objects.equals(setting.getApiValue(), owned)) {
            setting.clearValue();
            HorizonLab.LOGGER.info("Adaptive terrain override released; effective runtime ratio: {}", setting.getValue());
        }
        owned = null;
        policy = setting == null ? null : new TerrainBudget(setting.getTrueValue());
        resetWindow();
    }

    private void resetWindow() { windowStart = 0; totalNs = 0; count = 0; late = 0; }

    String status() {
        return yielded ? "yielded to another mod" : owned == null ? "waiting for active gameplay"
            : "DH runtime ratio " + owned + "; adjustments " + changes;
    }
}
