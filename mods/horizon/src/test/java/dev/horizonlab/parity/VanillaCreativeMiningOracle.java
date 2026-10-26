package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import net.minecraft.core.component.DataComponents;
import net.minecraft.core.component.DataComponentMap;
import com.mojang.serialization.JsonOps;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.Tool;

/** Calls the actual item predicate; its unused block and level arguments are null. */
public final class VanillaCreativeMiningOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var defaults = JsonParser.parseString(Files.readString(Path.of(args[0]).getParent()
            .getParent().resolve("horizon-client/26.3/items.json"))).getAsJsonObject();
        for (var entry : defaults.entrySet()) {
            var item = BuiltInRegistries.ITEM.getValue(Identifier.parse(entry.getKey()));
            var components = DataComponentMap.builder();
            var tool = entry.getValue().getAsJsonObject().getAsJsonObject("minecraft:tool");
            if (tool != null) {
                // Only the restriction is queried here; mining-rule tag resolution is separate.
                tool = tool.deepCopy();
                tool.add("rules", new com.google.gson.JsonArray());
                components.set(DataComponents.TOOL, Tool.CODEC.parse(JsonOps.INSTANCE, tool).getOrThrow());
            }
            item.builtInRegistryHolder().bindComponents(components.build());
        }
        var unsafeClass = Class.forName("sun.misc.Unsafe");
        var unsafeField = unsafeClass.getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        var unsafe = unsafeField.get(null);
        var player = (ServerPlayer) unsafeClass.getMethod("allocateInstance", Class.class)
            .invoke(unsafe, ServerPlayer.class);
        var abilities = new Abilities();
        var field = Player.class.getDeclaredField("abilities");
        field.setAccessible(true);
        field.set(player, abilities);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var item = BuiltInRegistries.ITEM.getValue(Identifier.parse(row.get("item").getAsString()));
                var stack = new ItemStack(item, row.get("count").getAsInt());
                var mode = row.get("tool").getAsString();
                if (!mode.equals("default")) {
                    stack.set(DataComponents.TOOL, new Tool(List.of(), 1.0f, 1, !mode.equals("deny")));
                }
                abilities.instabuild = row.get("creative").getAsBoolean();
                // Use the base Item implementation: special debug-stick interactions are separate.
                out.println(net.minecraft.world.item.Items.STICK.canDestroyBlock(stack, null, null, null, player));
            }
        }
    }
}
