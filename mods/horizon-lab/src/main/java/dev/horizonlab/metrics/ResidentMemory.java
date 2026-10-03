package dev.horizonlab.metrics;

import java.util.function.LongSupplier;
import oshi.SystemInfo;
import oshi.software.os.OSProcess;

/** Uses Minecraft's existing OSHI dependency; initialization stays on the sampler thread. */
public final class ResidentMemory implements LongSupplier {
    private OSProcess process;
    private boolean initialized;

    @Override public long getAsLong() {
        if (!initialized) {
            initialized = true;
            var os = new SystemInfo().getOperatingSystem();
            process = os.getProcess(os.getProcessId());
        }
        return process != null && process.updateAttributes() ? process.getResidentSetSize() : -1;
    }
}
