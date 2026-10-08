package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.heightproviders.HeightProvider;

/** Original transport calling native height-provider codecs and samples. */
public final class VanillaHeightProviderOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        var contextMethod=VanillaAnchorOracle.class.getDeclaredMethod("context",JsonObject.class);contextMethod.setAccessible(true);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                var context=(WorldGenerationContext)contextMethod.invoke(null,f);
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                com.mojang.serialization.DataResult<HeightProvider> decoded;
                try {decoded=HeightProvider.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(f.get("definition").getAsString()));} catch(IllegalArgumentException error) {out.println("decode error");out.println("tail "+random.nextLong());continue;}
                int count=f.get("samples").getAsInt();
                if(decoded.result().isEmpty())out.println("decode error");else {
                    var provider=decoded.result().get();out.println("decode ok");
                    for(int i=0;i<count;i++){try{out.println("sample "+provider.sample(random,context));}catch(IllegalArgumentException error){out.println("sample error");}out.println("next "+random.nextLong());}
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
