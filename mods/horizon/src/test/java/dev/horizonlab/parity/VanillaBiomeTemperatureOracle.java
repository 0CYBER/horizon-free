package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.concurrent.atomic.AtomicReference;
import it.unimi.dsi.fastutil.longs.Long2FloatLinkedOpenHashMap;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.biome.Biome;

/** Original transport for native biome temperature methods and cache contents. */
public final class VanillaBiomeTemperatureOracle {
    private static Method adjusted,temperature;
    private static Field cache;
    private static Object climate(JsonObject f) throws Exception {
        var type=Class.forName("net.minecraft.world.level.biome.Biome$ClimateSettings");
        if(f.has("json")) {
            var field=type.getDeclaredField("CODEC");field.setAccessible(true);
            var codec=(MapCodec<?>)field.get(null);
            return codec.codec().parse(JsonOps.INSTANCE,JsonParser.parseString(f.get("json").getAsString())).result().orElse(null);
        }
        var constructor=type.getDeclaredConstructor(boolean.class,float.class,Biome.TemperatureModifier.class,float.class);constructor.setAccessible(true);
        return constructor.newInstance(f.get("precipitation").getAsBoolean(),Float.intBitsToFloat(Integer.parseUnsignedInt(f.get("base").getAsString(),16)),Biome.TemperatureModifier.valueOf(f.get("modifier").getAsString()),f.has("downfall")?Float.intBitsToFloat(Integer.parseUnsignedInt(f.get("downfall").getAsString(),16)):0f);
    }
    private static Object field(Object value,String name) throws Exception {
        var method=value.getClass().getDeclaredMethod(name);method.setAccessible(true);return method.invoke(value);
    }
    private static Biome make(Object climate) throws Exception {
        var biome=Biome.class.getDeclaredConstructors()[0];biome.setAccessible(true);return (Biome)biome.newInstance(climate,null,null,null);
    }
    private static String bits(float value){return Integer.toUnsignedString(Float.floatToIntBits(value),16);}
    private static void operations(Biome biome,JsonArray operations,PrintWriter out) throws Exception {
        for(var item:operations) {
            var op=item.getAsJsonArray();switch(op.get(0).getAsString()) {
                case "query"->{
                    var pos=new BlockPos(op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsInt());int sea=op.get(4).getAsInt();
                    out.println("adjusted "+bits((Float)adjusted.invoke(biome,pos,sea)));
                    out.println("cached "+bits((Float)temperature.invoke(biome,pos,sea)));
                    out.println("weather "+biome.warmEnoughToRain(pos,sea)+" "+biome.coldEnoughToSnow(pos,sea)+" "+biome.shouldMeltFrozenOceanIcebergSlightly(pos,sea)+" "+biome.getPrecipitationAt(pos,sea));
                }
                case "fill"->{for(int i=0;i<op.get(1).getAsInt();i++){var pos=new BlockPos(op.get(2).getAsInt()+i,i&255,i*31);out.println("fill "+bits((Float)temperature.invoke(biome,pos,op.get(3).getAsInt())));}}
                case "dump"->{var map=(Long2FloatLinkedOpenHashMap)((ThreadLocal<?>)cache.get(biome)).get();out.println("cache "+map.size());for(var entry:map.long2FloatEntrySet())out.println(Long.toUnsignedString(entry.getLongKey(),16)+" "+bits(entry.getFloatValue()));}
                case "thread"->{
                    var failure=new AtomicReference<Throwable>();
                    var thread=new Thread(()->{try{operations(biome,op.get(1).getAsJsonArray(),out);}catch(Throwable error){failure.set(error);}});
                    thread.start();thread.join();if(failure.get()!=null)throw new AssertionError("temperature worker failed",failure.get());
                }
                default->throw new AssertionError("unknown operation "+op);
            }
        }
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        adjusted=Biome.class.getDeclaredMethod("getHeightAdjustedTemperature",BlockPos.class,int.class);adjusted.setAccessible(true);
        temperature=Biome.class.getDeclaredMethod("getTemperature",BlockPos.class,int.class);temperature.setAccessible(true);
        cache=Biome.class.getDeclaredField("temperatureCache");cache.setAccessible(true);
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();var climate=climate(f);
                if(climate==null){out.println("decode error");continue;}
                out.println("settings "+field(climate,"hasPrecipitation")+" "+bits((Float)field(climate,"temperature"))+" "+field(climate,"temperatureModifier")+" "+bits((Float)field(climate,"downfall")));
                operations(make(climate),f.getAsJsonArray("ops"),out);
            }
        }
    }
}
