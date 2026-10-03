package dev.horizonlab.mixin;

import java.nio.file.Files;
import java.util.List;
import java.util.Properties;
import java.util.Set;
import net.fabricmc.loader.api.FabricLoader;
import org.objectweb.asm.tree.ClassNode;
import org.slf4j.LoggerFactory;
import org.spongepowered.asm.mixin.extensibility.IMixinConfigPlugin;
import org.spongepowered.asm.mixin.extensibility.IMixinInfo;

/** The internal worker hook is opt-in and limited to the inspected, pinned DH version. */
public final class HorizonMixinPlugin implements IMixinConfigPlugin {
    private static boolean workerPacing;

    @Override public void onLoad(String mixinPackage) {
        var loader = FabricLoader.getInstance();
        var file = loader.getConfigDir().resolve("horizon-worker-pacing.properties");
        try {
            Properties properties = new Properties();
            if (Files.exists(file)) {
                try (var reader = Files.newBufferedReader(file)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false);
            workerPacing = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "Precise DH worker pacing: {} (requires pinned DH 3.3.4)", workerPacing ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Worker pacing disabled: invalid configuration", error);
        }
    }

    public static boolean workerPacingEnabled() { return workerPacing; }
    @Override public boolean shouldApplyMixin(String targetClassName, String mixinClassName) {
        return !mixinClassName.endsWith("DhWorkerPacingMixin") || workerPacing;
    }
    @Override public String getRefMapperConfig() { return null; }
    @Override public List<String> getMixins() { return null; }
    @Override public void acceptTargets(Set<String> mine, Set<String> others) {}
    @Override public void preApply(String name, ClassNode node, String mixin, IMixinInfo info) {}
    @Override public void postApply(String name, ClassNode node, String mixin, IMixinInfo info) {}
}
