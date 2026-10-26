package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Optional;
import net.minecraft.core.component.DataComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.Style;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.component.ItemLore;
import net.minecraft.world.item.component.TooltipDisplay;

/**
 * The styled lines {@code ItemLore.addToTooltip} adds, one stack (SNBT) per input
 * line. Each line is its visited spans ({@code text, colour, bold, italic,
 * underlined, strikethrough, obfuscated}); lines are separated by {@code ||}.
 */
public final class VanillaLoreTooltipOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var context = Item.TooltipContext.of(ParityData.lookup);
        var output = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank() || line.startsWith("#")) continue;
            var stack = ParityData.stack(line);
            var display = stack.getOrDefault(DataComponents.TOOLTIP_DISPLAY, TooltipDisplay.DEFAULT);
            ItemLore lore = stack.get(DataComponents.LORE);
            if (lore == null || !display.shows(DataComponents.LORE)) {
                output.add("-");
                continue;
            }
            var lines = new ArrayList<String>();
            lore.addToTooltip(context, component -> lines.add(spans(component)), TooltipFlag.NORMAL, stack);
            output.add(lines.isEmpty() ? "-" : String.join(" || ", lines));
        }
        Files.write(Path.of(args[1]), output);
    }

    /** {@code Component.visit} from {@code Style.EMPTY}: the style a tooltip draws. */
    static String spans(Component component) {
        var out = new StringBuilder();
        component.visit((style, text) -> {
            if (text.isEmpty()) return Optional.empty();
            if (out.length() > 0) out.append(" | ");
            int color = style.getColor() == null ? -1 : style.getColor().getValue();
            out.append(text).append('\t').append(color).append('\t')
                .append(style.isBold()).append('\t')
                .append(style.isItalic()).append('\t')
                .append(style.isUnderlined()).append('\t')
                .append(style.isStrikethrough()).append('\t')
                .append(style.isObfuscated());
            return Optional.empty();
        }, Style.EMPTY);
        return out.toString();
    }
}
