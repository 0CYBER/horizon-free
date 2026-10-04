package dev.horizonlab.metrics;

import java.util.LinkedHashMap;
import java.util.Map;
import net.fabricmc.loader.api.FabricLoader;

/** Software identity captured once before a client or server recording starts. */
public final class SoftwareContext {
    private SoftwareContext() {}

    public static Map<String, Object> capture() {
        Map<String, Object> context = new LinkedHashMap<>();
        context.put("java_vendor", System.getProperty("java.vendor"));
        context.put("java_vm_name", System.getProperty("java.vm.name"));
        context.put("java_runtime_version", System.getProperty("java.runtime.version"));
        context.put("mods", FabricLoader.getInstance().getAllMods().stream()
            .map(mod -> mod.getMetadata().getId() + "@" + mod.getMetadata().getVersion().getFriendlyString())
            .sorted().toList());
        return context;
    }
}
