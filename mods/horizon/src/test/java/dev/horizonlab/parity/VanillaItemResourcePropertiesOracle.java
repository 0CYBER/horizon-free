package dev.horizonlab.parity;

import com.google.gson.*;
import java.nio.file.*;
import java.util.Locale;
import net.minecraft.resources.RegistryOps;
import net.minecraft.world.item.ItemStack;

/** Effective stack reads over the real vanilla registries and component defaults. */
public final class VanillaItemResourcePropertiesOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var ops = RegistryOps.create(com.mojang.serialization.JsonOps.INSTANCE, ParityData.lookup);
        try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                if (line.isBlank()) continue;
                var input = JsonParser.parseString(line).getAsJsonObject();
                var stack = ItemStack.CODEC.parse(ops, input.get("stack")).getOrThrow();
                var materialInput = new JsonObject();
                materialInput.addProperty("id", input.get("repair").getAsString());
                var material = ItemStack.CODEC.parse(ops, materialInput).getOrThrow();
                var result = new JsonObject();
                result.addProperty("name", input.get("name").getAsString());
                result.addProperty("damageable", stack.isDamageableItem());
                result.addProperty("foil", stack.hasFoil());
                result.addProperty("use_duration", stack.getUseDuration(null));
                result.addProperty("rarity", stack.getRarity().name().toLowerCase(Locale.ROOT));
                result.addProperty("repairable", stack.isValidRepairItem(material));
                result.addProperty("enchantable", stack.isEnchantable());
                if (stack.isBarVisible()) {
                    var bar = new JsonObject();
                    bar.addProperty("width", stack.getBarWidth());
                    // ItemBar exposes RGB, independent of the item implementation's alpha bits.
                    bar.addProperty("color", stack.getBarColor() & 0xFFFFFF);
                    result.add("bar", bar);
                } else result.add("bar", JsonNull.INSTANCE);
                output.write(result.toString());
                output.newLine();
            }
        }
    }
}
