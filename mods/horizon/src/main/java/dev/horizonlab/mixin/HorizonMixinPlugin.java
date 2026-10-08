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

/** Internal hooks are opt-in and limited to the inspected, pinned DH version. */
public final class HorizonMixinPlugin implements IMixinConfigPlugin {
    private static boolean quadReuse;
    private static boolean terrainPosition;
    private static boolean workerPacing;
    private static boolean debounceWait;
    private static boolean dispatchRetry;
    private static boolean tickDedup;
    private static boolean cullingEmpty;
    private static boolean terrainReadWait;
    private static boolean cullingRayScratch;

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
        try {
            Properties properties = new Properties();
            var queueFile = loader.getConfigDir().resolve("horizon-queue-wait.properties");
            if (Files.exists(queueFile)) {
                try (var reader = Files.newBufferedReader(queueFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false);
            debounceWait = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "DH debounce queue wait: {} (requires pinned DH 3.3.4)", debounceWait ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Debounce queue wait disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var dispatchFile = loader.getConfigDir().resolve("horizon-dispatch-retry.properties");
            if (Files.exists(dispatchFile)) {
                try (var reader = Files.newBufferedReader(dispatchFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false);
            dispatchRetry = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "DH rejected dispatch retry: {} (requires pinned DH 3.3.4)", dispatchRetry ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Dispatch retry disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var dedupFile = loader.getConfigDir().resolve("horizon-tick-dedup.properties");
            if (Files.exists(dedupFile)) {
                try (var reader = Files.newBufferedReader(dedupFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false);
            tickDedup = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "DH tick membership deduplication: {} (requires pinned DH 3.3.4)", tickDedup ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Tick membership deduplication disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var cullingFile = loader.getConfigDir().resolve("horizon-culling-empty.properties");
            if (Files.exists(cullingFile)) {
                try (var reader = Files.newBufferedReader(cullingFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("entityculling")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("1.11.2")).orElse(false);
            cullingEmpty = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "Empty culling whitelist iteration: {} (requires pinned Entity Culling 1.11.2)", cullingEmpty ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Empty culling whitelist iteration disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var readFile = loader.getConfigDir().resolve("horizon-terrain-read-wait.properties");
            if (Files.exists(readFile)) {
                try (var reader = Files.newBufferedReader(readFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false)
                && loader.getModContainer("minecraft")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("26.3")).orElse(false);
            terrainReadWait = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "Interruptible DH blending metadata reads: {} (requires Minecraft 26.3 and DH 3.3.4)",
                terrainReadWait ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Interruptible terrain read disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var rayFile = loader.getConfigDir().resolve("horizon-culling-rays.properties");
            if (Files.exists(rayFile)) {
                try (var reader = Files.newBufferedReader(rayFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("entityculling")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("1.11.2")).orElse(false);
            cullingRayScratch = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "Culling ray scratch vectors: {} (requires pinned Entity Culling 1.11.2)", cullingRayScratch ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Culling ray scratch vectors disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var positionFile = loader.getConfigDir().resolve("horizon-terrain-position.properties");
            if (Files.exists(positionFile)) {
                try (var reader = Files.newBufferedReader(positionFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false);
            terrainPosition = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "Relative terrain position arithmetic: {} (requires pinned DH 3.3.4)", terrainPosition ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Terrain position arithmetic disabled: invalid configuration", error);
        }
        try {
            Properties properties = new Properties();
            var quadFile = loader.getConfigDir().resolve("horizon-quad-reuse.properties");
            if (Files.exists(quadFile)) {
                try (var reader = Files.newBufferedReader(quadFile)) { properties.load(reader); }
            }
            boolean requested = Boolean.parseBoolean(properties.getProperty("enabled", "false"));
            boolean supported = loader.getModContainer("distanthorizons")
                .map(mod -> mod.getMetadata().getVersion().getFriendlyString().equals("3.3.4")).orElse(false);
            quadReuse = requested && supported;
            if (requested) LoggerFactory.getLogger("horizonlab").info(
                "Discarded terrain quad reuse: {} (requires pinned DH 3.3.4)", quadReuse ? "enabled" : "unavailable");
        } catch (Exception error) {
            LoggerFactory.getLogger("horizonlab").warn("Terrain quad reuse disabled: invalid configuration", error);
        }
    }

    public static boolean quadReuseEnabled() { return quadReuse; }
    public static boolean terrainPositionEnabled() { return terrainPosition; }
    public static boolean workerPacingEnabled() { return workerPacing; }
    public static boolean debounceWaitEnabled() { return debounceWait; }
    public static boolean dispatchRetryEnabled() { return dispatchRetry; }
    public static boolean tickDedupEnabled() { return tickDedup; }
    public static boolean cullingEmptyEnabled() { return cullingEmpty; }
    public static boolean cullingRayScratchEnabled() { return cullingRayScratch; }
    public static boolean terrainReadWaitEnabled() { return terrainReadWait; }
    @Override public boolean shouldApplyMixin(String targetClassName, String mixinClassName) {
        if (mixinClassName.endsWith("DhWorkerPacingMixin")) return workerPacing;
        if (mixinClassName.endsWith("DhDebounceWaitMixin")) return debounceWait;
        if (mixinClassName.endsWith("DhPriorityDispatchMixin")) return dispatchRetry;
        if (mixinClassName.endsWith("DhTickDedupMixin")) return tickDedup;
        if (mixinClassName.endsWith("CullingEmptyWhitelistMixin")) return cullingEmpty;
        if (mixinClassName.endsWith("CullingRayScratchMixin")) return cullingRayScratch;
        if (mixinClassName.endsWith("DhTerrainReadWaitMixin")) return terrainReadWait;
        if (mixinClassName.endsWith("DhRelativePositionMixin")) return terrainPosition;
        if (mixinClassName.endsWith("DhQuadReuseMixin")) return quadReuse;
        return true;
    }
    @Override public String getRefMapperConfig() { return null; }
    @Override public List<String> getMixins() { return null; }
    @Override public void acceptTargets(Set<String> mine, Set<String> others) {}
    @Override public void preApply(String name, ClassNode node, String mixin, IMixinInfo info) {}
    @Override public void postApply(String name, ClassNode node, String mixin, IMixinInfo info) {}
}
