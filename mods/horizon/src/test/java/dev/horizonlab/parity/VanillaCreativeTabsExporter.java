package dev.horizonlab.parity;

import com.google.gson.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.CreativeModeTabs;

/**
 * Original exporter of the creative inventory tab contents built by vanilla's own
 * CreativeModeTab display generators (built-in registries; tabs needing dynamic
 * registries keep whatever they produced before failing).
 */
public final class VanillaCreativeTabsExporter {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        // Display generators only need ItemStacks to exist: bind minimal components.
        for (var item : BuiltInRegistries.ITEM) {
            item.builtInRegistryHolder().bindComponents(
                net.minecraft.core.component.DataComponentMap.builder()
                    .set(net.minecraft.core.component.DataComponents.MAX_STACK_SIZE, 64)
                    .build());
        }
        var lookup = RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        var parameters = new CreativeModeTab.ItemDisplayParameters(FeatureFlags.DEFAULT_FLAGS, true, lookup);
        var out = new JsonArray();
        for (CreativeModeTab tab : BuiltInRegistries.CREATIVE_MODE_TAB) {
            var o = new JsonObject();
            var key = BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab);
            o.addProperty("id", key == null ? "" : key.toString());
            o.addProperty("type", tab.getType().name());
            o.addProperty("row", tab.row().name());
            o.addProperty("column", tab.column());
            o.addProperty("title", tab.getDisplayName().getString());
            var items = new JsonArray();
            try {
                tab.buildContents(parameters);
                for (var stack : tab.getDisplayItems()) {
                    items.add(BuiltInRegistries.ITEM.getKey(stack.getItem()).toString());
                }
            } catch (RuntimeException error) {
                o.addProperty("error", error.toString());
            }
            try {
                o.addProperty("icon", BuiltInRegistries.ITEM.getKey(tab.getIconItem().getItem()).toString());
            } catch (RuntimeException ignored) {
            }
            o.add("items", items);
            out.add(o);
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(out), StandardCharsets.UTF_8);
    }
}
