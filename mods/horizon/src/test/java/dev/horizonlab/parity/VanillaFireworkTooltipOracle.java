package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.component.TooltipDisplay;
import net.minecraft.world.item.component.TooltipProvider;

/**
 * The tooltip lines of a stack's fireworks and firework_explosion components, as
 * ItemStack.addDetailsToTooltip adds them (Fireworks / FireworkExplosion.addToTooltip
 * unless its tooltip_display hides them), one stack (SNBT) per input line.
 */
public final class VanillaFireworkTooltipOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var context = Item.TooltipContext.of(ParityData.lookup);
        List<DataComponentType<? extends TooltipProvider>> types = List.of(DataComponents.FIREWORKS, DataComponents.FIREWORK_EXPLOSION);
        var output = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.startsWith("#")) continue;
            var stack = ParityData.stack(line);
            var display = stack.getOrDefault(DataComponents.TOOLTIP_DISPLAY, TooltipDisplay.DEFAULT);
            var lines = new ArrayList<String>();
            for (var type : types) {
                TooltipProvider provider = stack.get(type);
                if (provider != null && display.shows(type)) {
                    provider.addToTooltip(context, c -> lines.add(c.getString()), TooltipFlag.NORMAL, stack);
                }
            }
            output.add(lines.isEmpty() ? "-" : String.join(" | ", lines));
        }
        Files.write(Path.of(args[1]), output);
    }
}
