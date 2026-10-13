package dev.horizonlab.parity;

import com.google.gson.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.List;
import net.minecraft.ChatFormatting;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.chat.Component;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.CreativeModeTab;
import net.minecraft.world.item.CreativeModeTabs;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TooltipFlag;

/**
 * Original exporter of the creative inventory: every tab's contents as vanilla's
 * own CreativeModeTabs.tryRebuildTabContents builds them over the vanilla data
 * pack's registries (category tabs first, then the search tab from their
 * search-tab items), each stack as SNBT with its components, and for every stack
 * of the search tab the inputs of the client's creative search trees
 * (SessionSearchTrees): its creative tooltip lines and its item tags.
 *
 *   VanillaCreativeTabsExporter OUT.json
 */
public final class VanillaCreativeTabsExporter {
    /** The operator tab and its items need permissions and the operatorItemsTab option (off by default). */
    public static final boolean HAS_PERMISSIONS = false;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        Files.writeString(Path.of(args[0]), export(FeatureFlags.DEFAULT_FLAGS, HAS_PERMISSIONS, ParityData.lookup).toString(),
            StandardCharsets.UTF_8);
    }

    public static JsonObject export(FeatureFlagSet features, boolean hasPermissions, HolderLookup.Provider lookup) {
        CreativeModeTabs.tryRebuildTabContents(features, hasPermissions, lookup);
        var tabs = new JsonArray();
        for (CreativeModeTab tab : BuiltInRegistries.CREATIVE_MODE_TAB) {
            var o = new JsonObject();
            var key = BuiltInRegistries.CREATIVE_MODE_TAB.getKey(tab);
            o.addProperty("id", key == null ? "" : key.toString());
            o.addProperty("type", tab.getType().name());
            o.addProperty("row", tab.row().name());
            o.addProperty("column", tab.column());
            o.addProperty("title", tab.getDisplayName().getString());
            o.addProperty("icon", snbt(tab.getIconItem()));
            o.addProperty("visible", tab.shouldDisplay());
            var items = new JsonArray();
            for (ItemStack stack : tab.getDisplayItems()) items.add(snbt(stack));
            o.add("items", items);
            if (tab.getType() == CreativeModeTab.Type.SEARCH) {
                var context = Item.TooltipContext.of(lookup);
                var flag = TooltipFlag.Default.NORMAL.asCreative();
                var lines = new JsonArray();
                var tags = new JsonArray();
                for (ItemStack stack : tab.getDisplayItems()) {
                    lines.add(strings(tooltipLines(stack, context, flag)));
                    tags.add(strings(stack.tags().map(t -> t.location().toString()).toList()));
                }
                o.add("lines", lines);
                o.add("tags", tags);
            }
            tabs.add(o);
        }
        var out = new JsonObject();
        out.addProperty("schema", 2);
        out.addProperty("permissions", hasPermissions);
        out.add("tabs", tabs);
        return out;
    }

    /** SessionSearchTrees.getTooltipLines for one stack. */
    public static List<String> tooltipLines(ItemStack stack, Item.TooltipContext context, TooltipFlag flag) {
        return stack.getTooltipLines(context, null, flag).stream()
            .map(Component::getString)
            .map(l -> ChatFormatting.stripFormatting(l).trim())
            .filter(s -> !s.isEmpty())
            .toList();
    }

    /** ItemStack.CODEC as SNBT (the form horizon_client::item_data::parse_stack reads). */
    public static String snbt(ItemStack stack) {
        if (stack.isEmpty()) return "-";
        return ItemStack.CODEC.encodeStart(ParityData.ops, stack).getOrThrow().toString();
    }

    private static JsonArray strings(List<String> values) {
        var a = new JsonArray();
        values.forEach(a::add);
        return a;
    }
}
