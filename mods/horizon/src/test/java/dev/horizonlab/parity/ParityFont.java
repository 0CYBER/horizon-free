package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import com.mojang.blaze3d.font.GlyphInfo;
import com.mojang.blaze3d.font.GlyphProvider;
import com.mojang.blaze3d.font.UnbakedGlyph;
import com.mojang.serialization.JsonOps;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.HashSet;
import java.util.Set;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GlyphSource;
import net.minecraft.client.gui.font.glyphs.BakedGlyph;
import net.minecraft.client.gui.font.providers.GlyphProviderDefinition;
import net.minecraft.client.gui.font.TextRenderable;
import net.minecraft.network.chat.FontDescription;
import net.minecraft.network.chat.Style;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.RandomSource;

/**
 * The vanilla default font's glyph metrics without a GPU: the `space` and
 * `include/default` providers of the client jar loaded by vanilla's own
 * provider definitions (BitmapProvider scans ascii.png etc. for advances).
 * Glyphs carry metrics only; nothing is stitched.
 */
public final class ParityFont {
    public static ResourceManager resources() throws Exception {
        return resources(Path.of("/nonexistent-horizon-assets"));
    }

    public static ResourceManager resources(Path assets) throws Exception {
        var method = net.minecraft.client.resources.ClientPackSource.class.getDeclaredMethod("createVanillaPackSource", Path.class);
        method.setAccessible(true);
        var pack = (net.minecraft.server.packs.VanillaPackResources) method.invoke(null, assets);
        return pack.asResourceManager();
    }

    public static Font create(ResourceManager resources) throws Exception {
        List<GlyphProvider> providers = new ArrayList<>();
        for (String include : new String[] {"space", "default"}) {
            var id = Identifier.withDefaultNamespace("font/include/" + include + ".json");
            try (var reader = new InputStreamReader(resources.open(id), StandardCharsets.UTF_8)) {
                var json = JsonParser.parseReader(reader).getAsJsonObject();
                for (var element : json.getAsJsonArray("providers")) {
                    var conditional = GlyphProviderDefinition.Conditional.CODEC.parse(JsonOps.INSTANCE, element).getOrThrow();
                    var loader = conditional.definition().unpack().left().orElseThrow();
                    providers.add(loader.load(resources));
                }
            }
        }
        return create(providers);
    }

    /** The full default font, including references and the external Unihex fallback. */
    public static Font createDefault(ResourceManager resources) throws Exception {
        List<GlyphProvider> providers = new ArrayList<>();
        loadProviders(resources, Identifier.withDefaultNamespace("default"), providers, new HashSet<>());
        return create(providers);
    }

    private static void loadProviders(ResourceManager resources, Identifier font, List<GlyphProvider> providers,
                                      Set<Identifier> visiting) throws Exception {
        if (!visiting.add(font)) throw new IllegalArgumentException("Cyclic font reference: " + font);
        var file = font.withPath("font/" + font.getPath() + ".json");
        try (var reader = new InputStreamReader(resources.open(file), StandardCharsets.UTF_8)) {
            var json = JsonParser.parseReader(reader).getAsJsonObject();
            for (var element : json.getAsJsonArray("providers")) {
                var conditional = GlyphProviderDefinition.Conditional.CODEC.parse(JsonOps.INSTANCE, element).getOrThrow();
                if (!conditional.filter().apply(Set.of())) continue;
                var definition = conditional.definition().unpack();
                if (definition.left().isPresent()) {
                    providers.add(definition.left().orElseThrow().load(resources));
                } else {
                    loadProviders(resources, definition.right().orElseThrow().id(), providers, visiting);
                }
            }
        } finally {
            visiting.remove(font);
        }
    }

    private static Font create(List<GlyphProvider> providers) {
        GlyphSource source = new GlyphSource() {
            @Override
            public BakedGlyph getGlyph(int codepoint) {
                for (GlyphProvider provider : providers) {
                    UnbakedGlyph glyph = provider.getGlyph(codepoint);
                    if (glyph != null) return baked(glyph.info());
                }
                return baked(GlyphInfo.simple(6.0F));
            }

            @Override
            public BakedGlyph getRandomGlyph(RandomSource random, int width) {
                return getGlyph('?');
            }
        };
        return new Font(new Font.Provider() {
            @Override
            public GlyphSource glyphs(FontDescription font) {
                return source;
            }

            @Override
            public net.minecraft.client.gui.font.glyphs.EffectGlyph effect() {
                return null;
            }
        });
    }

    private static BakedGlyph baked(GlyphInfo info) {
        return new BakedGlyph() {
            @Override
            public GlyphInfo info() {
                return info;
            }

            @Override
            public TextRenderable.Styled createGlyph(float x, float y, int color, int shadowColor, Style style, float boldOffset, float shadowOffset) {
                return null;
            }
        };
    }
}
