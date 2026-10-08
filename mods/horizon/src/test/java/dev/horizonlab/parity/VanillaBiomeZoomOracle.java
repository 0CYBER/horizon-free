package dev.horizonlab.parity;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.biome.BiomeManager;

/** Original transport; actual biome manager selects coordinates for a traced resolver. */
public final class VanillaBiomeZoomOracle {
    public static void main(String[] args) throws Exception {
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var v=line.split(" ");
                switch(v[0]) {
                    case "s" -> out.println("s "+BiomeManager.obfuscateSeed(Long.parseLong(v[1])));
                    case "b" -> {
                        out.print("b");
                        var manager=new BiomeManager((x,y,z)->{out.print(" "+x+" "+y+" "+z);return null;},Long.parseLong(v[1]));
                        var pos=new BlockPos(Integer.parseInt(v[2]),Integer.parseInt(v[3]),Integer.parseInt(v[4]));
                        manager.getBiome(pos);manager.getNoiseBiomeAtPosition(pos);
                        manager.withDifferentSource((x,y,z)->{out.print(" "+x+" "+y+" "+z);return null;}).getBiome(pos.getX(),pos.getY(),pos.getZ());
                        out.println();
                    }
                    case "p" -> {
                        out.print("p");
                        var manager=new BiomeManager((x,y,z)->{out.print(" "+x+" "+y+" "+z);return null;},0);
                        manager.getNoiseBiomeAtPosition(Double.longBitsToDouble(Long.parseUnsignedLong(v[1],16)),Double.longBitsToDouble(Long.parseUnsignedLong(v[2],16)),Double.longBitsToDouble(Long.parseUnsignedLong(v[3],16)));
                        out.println();
                    }
                    default -> throw new IllegalArgumentException("unknown fixture operation");
                }
            }
        }
    }
}
