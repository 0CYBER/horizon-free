package dev.horizonlab.parity;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.blaze3d.platform.NativeImage;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Locale;
import net.minecraft.client.renderer.texture.MipmapGenerator;
import net.minecraft.client.renderer.texture.MipmapStrategy;
import net.minecraft.resources.Identifier;

/**
 * Vanilla's sprite mip levels for tools/test_mipmap_parity.py. The first
 * input line is the resources root; each further line is a sprite
 * (`minecraft:block/cactus_side`). The oracle reads its PNG and the `texture`
 * section of its .mcmeta (mipmap_strategy, alpha_cutoff_bias), as
 * SpriteContents does, and prints every level MipmapGenerator.generateMipLevels
 * makes (down to 1x1) as RGBA hex rows.
 */
public final class VanillaMipmapOracle {
    public static void main(String[] args) throws Exception {
        List<String> lines = Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8);
        Path root = Path.of(lines.get(0));
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
            for (String name : lines.subList(1, lines.size())) {
                Identifier id = Identifier.parse(name);
                Path png = root.resolve("assets/" + id.getNamespace() + "/textures/" + id.getPath() + ".png");
                Path meta = png.resolveSibling(png.getFileName() + ".mcmeta");
                MipmapStrategy strategy = MipmapStrategy.AUTO;
                float bias = 0.0F;
                if (Files.isRegularFile(meta)) {
                    JsonObject json = JsonParser.parseString(Files.readString(meta)).getAsJsonObject();
                    if (json.has("texture")) {
                        JsonObject texture = json.getAsJsonObject("texture");
                        if (texture.has("mipmap_strategy"))
                            strategy = MipmapStrategy.valueOf(texture.get("mipmap_strategy").getAsString().toUpperCase(Locale.ROOT));
                        if (texture.has("alpha_cutoff_bias")) bias = texture.get("alpha_cutoff_bias").getAsFloat();
                    }
                }
                NativeImage image;
                try (var in = Files.newInputStream(png)) {
                    image = NativeImage.read(in);
                }
                int levels = Integer.numberOfTrailingZeros(Math.min(image.getWidth(), image.getHeight()));
                NativeImage[] mips = MipmapGenerator.generateMipLevels(id, new NativeImage[] {image}, levels, strategy, bias, image.computeTransparency());
                out.println("== " + name);
                for (NativeImage mip : mips) {
                    StringBuilder line = new StringBuilder(mip.getWidth() + "x" + mip.getHeight());
                    for (int y = 0; y < mip.getHeight(); y++) {
                        line.append(' ');
                        for (int x = 0; x < mip.getWidth(); x++) {
                            int c = mip.getPixel(x, y);
                            line.append(String.format(Locale.ROOT, "%02x%02x%02x%02x", c >> 16 & 255, c >> 8 & 255, c & 255, c >>> 24));
                        }
                    }
                    out.println(line);
                }
            }
        }
    }
}
