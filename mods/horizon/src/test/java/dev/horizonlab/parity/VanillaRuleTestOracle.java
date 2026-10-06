package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.structure.templatesystem.RuleTest;

/** Exercises native replacement-rule codecs and ordered random evaluation. */
public final class VanillaRuleTestOracle {
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaBlockStateCodecOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)inputs::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                var decoded=RuleTest.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(f.get("definition").getAsString())).result();
                if(decoded.isEmpty())out.println("decode error");else {
                    out.println("decode ok");int index=0;var heights=f.getAsJsonArray("heights");
                    for(int repeat=0;repeat<f.get("repeats").getAsInt();repeat++)for(var state:f.getAsJsonArray("states")) {
                        int y=heights.get(index++%heights.size()).getAsInt();
                        boolean result=decoded.get().test(Block.stateById(state.getAsInt()),new BlockPos(17,y,-33),random);
                        out.println("test "+result+" "+random.nextLong());
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
