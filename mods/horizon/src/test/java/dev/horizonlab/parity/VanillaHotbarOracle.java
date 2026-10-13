package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.HotbarManager;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtIo;
import net.minecraft.nbt.TagParser;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.item.ItemStack;

/**
 * Original oracle for creative toolbars read from a hotbar.nbt of any version:
 * each case's SNBT is written as the file and the real HotbarManager (with the
 * game's DataFixer, so DataFixTypes.HOTBAR and its item stack fixers) loads it;
 * each of the nine rows is then loaded with Hotbar.load and printed.
 */
public final class VanillaHotbarOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        Path dir = Files.createTempDirectory("hotbar");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                try {
                    Files.deleteIfExists(dir.resolve("hotbar.nbt"));
                    if (!f.get("snbt").isJsonNull()) {
                        CompoundTag tag = TagParser.parseCompoundFully(f.get("snbt").getAsString());
                        NbtIo.write(tag, dir.resolve("hotbar.nbt"));
                    }
                    HotbarManager manager = new HotbarManager(dir, DataFixers.getDataFixer());
                    StringBuilder result = new StringBuilder();
                    for (int i = 0; i < HotbarManager.NUM_HOTBAR_GROUPS; i++) {
                        result.append(i).append(':');
                        for (ItemStack stack : manager.get(i).load(ParityData.lookup)) result.append(' ').append(ParityData.canon(stack));
                        result.append(" | ");
                    }
                    out.println(result);
                } catch (Throwable e) {
                    out.println("error-case " + e.getClass().getSimpleName() + " " + e.getMessage());
                    if (System.getenv("MENU_TRACE") != null) e.printStackTrace();
                }
            }
        }
    }
}
