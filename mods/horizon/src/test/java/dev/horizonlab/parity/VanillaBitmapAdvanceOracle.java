package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import com.mojang.serialization.JsonOps;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.gui.font.providers.GlyphProviderDefinition;
import net.minecraft.network.chat.Component;
import net.minecraft.server.packs.PackLocationInfo;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.PathPackResources;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;

/** Real BitmapProvider.Definition loading of each provider in a resource tree, one advance per provider. */
public final class VanillaBitmapAdvanceOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        Path root = Path.of(args[0]);
        var info = new PackLocationInfo("parity", Component.literal("parity"), PackSource.BUILT_IN, Optional.empty());
        var resources = new MultiPackResourceManager(PackType.CLIENT_RESOURCES, List.of(new PathPackResources(info, root)));
        var json = JsonParser.parseString(Files.readString(root.resolve("assets/minecraft/font/default.json")))
            .getAsJsonObject().getAsJsonArray("providers");
        var output = new StringBuilder();
        int index = 0;
        for (var element : json) {
            var conditional = GlyphProviderDefinition.Conditional.CODEC.parse(JsonOps.INSTANCE, element).getOrThrow();
            var provider = conditional.definition().unpack().left().orElseThrow().load(resources);
            output.append(String.format(Locale.ROOT, "%.3f%n", provider.getGlyph(0xE000 + index++).info().getAdvance()));
        }
        Files.writeString(Path.of(args[1]), output);
    }
}
