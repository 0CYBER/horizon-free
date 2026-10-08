package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.IntegerProperty;

/** Original transport for actual compact/full block-state codecs. */
public final class VanillaBlockStateCodecOracle {
    private static JsonElement canonical(JsonElement value) {
        if(value.isJsonObject()){var result=new JsonObject();for(var e:new TreeMap<>(value.getAsJsonObject().asMap()).entrySet())result.add(e.getKey(),canonical(e.getValue()));return result;}
        if(value.isJsonArray()){var result=new JsonArray();for(var e:value.getAsJsonArray())result.add(canonical(e));return result;}
        return value;
    }
    private static String encode(BlockState state,boolean full){return canonical((full?BlockState.FULL_CODEC:BlockState.CODEC).encodeStart(JsonOps.INSTANCE,state).getOrThrow()).toString();}
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        if(args[0].equals("export")) {
            var result=new JsonObject();var states=new JsonArray();var defaults=new JsonObject();var integers=new JsonArray();
            for(var state:Block.BLOCK_STATE_REGISTRY) {
                if(Block.getId(state)!=states.size())throw new AssertionError("registry IDs not dense");
                states.add(BlockState.FULL_CODEC.encodeStart(JsonOps.INSTANCE,state).getOrThrow());
            }
            for(var id:new TreeSet<>(BuiltInRegistries.BLOCK.keySet())) {
                var block=BuiltInRegistries.BLOCK.getValue(id);defaults.addProperty(id.toString(),Block.getId(block.defaultBlockState()));
                for(var p:block.getStateDefinition().getProperties())if(p instanceof IntegerProperty){var entry=new JsonArray();entry.add(id.toString());entry.add(p.getName());integers.add(entry);}
            }
            result.add("states",states);result.add("defaults",defaults);result.add("integers",integers);
            Files.writeString(dir,result.toString(),StandardCharsets.UTF_8);return;
        }
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var state:Block.BLOCK_STATE_REGISTRY)out.println("state "+Block.getId(state)+" "+encode(state,false)+" "+encode(state,true));
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var input=JsonParser.parseString(line).getAsJsonArray();boolean full=input.get(0).getAsBoolean();
                var value=(full?BlockState.FULL_CODEC:BlockState.CODEC).parse(JsonOps.INSTANCE,JsonParser.parseString(input.get(1).getAsString())).result();
                if(value.isEmpty())out.println("decode error");else {var state=value.get();out.println("decode "+Block.getId(state)+" "+encode(state,false)+" "+encode(state,true));}
            }
        }
    }
}
