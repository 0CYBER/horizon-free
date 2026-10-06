package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.util.RandomSource;
import net.minecraft.util.valueproviders.*;
import net.minecraft.world.level.levelgen.*;

/** Original transport calling native value-provider codecs and samples. */
public final class VanillaValueProviderOracle {
    private static String bits(float value){return Integer.toHexString(Float.floatToIntBits(value));}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();long seed=f.get("seed").getAsLong();
                RandomSource random=f.get("legacy").getAsBoolean()?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
                var json=JsonParser.parseString(f.get("definition").getAsString());int count=f.get("samples").getAsInt();
                if(f.get("kind").getAsString().equals("float")) {
                    var decoded=FloatProviders.CODEC.parse(JsonOps.INSTANCE,json);
                    if(decoded.result().isEmpty())out.println("decode error");else {
                        var provider=decoded.result().get();out.println("float "+bits(provider.min())+" "+bits(provider.max()));
                        for(int i=0;i<count;i++){out.println("sample "+bits(provider.sample(random)));out.println("next "+random.nextLong());}
                    }
                } else {
                    com.mojang.serialization.DataResult<IntProvider> decoded;
                    try {decoded=IntProviders.CODEC.parse(JsonOps.INSTANCE,json);} catch(IllegalArgumentException error) {out.println("decode error");out.println("tail "+random.nextLong());continue;}
                    if(decoded.result().isEmpty())out.println("decode error");else {
                        var provider=decoded.result().get();out.println("int "+provider.minInclusive()+" "+provider.maxInclusive());
                        for(int i=0;i<count;i++){try{out.println("sample "+provider.sample(random));}catch(IllegalArgumentException error){out.println("sample error");}out.println("next "+random.nextLong());}
                    }
                }
                out.println("tail "+random.nextLong());
            }
        }
    }
}
