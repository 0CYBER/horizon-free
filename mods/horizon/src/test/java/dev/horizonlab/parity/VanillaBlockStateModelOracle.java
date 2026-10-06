package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.renderer.block.dispatch.*;
import net.minecraft.client.renderer.block.dispatch.multipart.*;
import net.minecraft.util.StrictJsonParser;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.map;

/** Original adapter around the actual blockstate resource parser and codec. */
public final class VanillaBlockStateModelOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static Object variant(Variant value) {
        var state=value.modelState();
        return map("model",value.modelLocation().toString(),"rotations",new int[]{state.x().ordinal(),state.y().ordinal(),state.z().ordinal()},"uvlock",state.uvLock());
    }
    private static Object model(BlockStateModel.Unbaked value) {
        if(value instanceof SingleVariant.Unbaked single)return map("single",variant(single.variant()));
        var weighted=(WeightedVariants.Unbaked)value;
        return map("weighted",weighted.entries().unwrap().stream().map(e->Arrays.asList(variant(((SingleVariant.Unbaked)e.value()).variant()),e.weight())).toList());
    }
    private static Object condition(Condition value) {
        if(value instanceof CombinedCondition combined)return map(combined.operation().name().toLowerCase(Locale.ROOT),combined.terms().stream().map(VanillaBlockStateModelOracle::condition).toList());
        var tests=new TreeMap<String,Object>();
        ((KeyValueCondition)value).tests().forEach((key,terms)->tests.put(key,terms.entries().stream().map(t->map("value",t.value(),"negated",t.negated())).toList()));
        return map("properties",tests);
    }
    private static Object definition(BlockStateModelDispatcher value) {
        var variants=value.simpleModels().map(m->m.models().entrySet().stream().map(e->Arrays.asList(e.getKey(),model(e.getValue()))).toList()).orElse(null);
        var multipart=value.multiPart().map(m->m.selectors().stream().map(s->map("when",s.condition().map(VanillaBlockStateModelOracle::condition).orElse(null),"apply",model(s.variant()))).toList()).orElse(null);
        return map("variants",variants,"multipart",multipart);
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        try(var input=Files.newBufferedReader(Path.of(args[0]));var output=Files.newBufferedWriter(Path.of(args[1]))) {
            for(String line;(line=input.readLine())!=null;) {
                var row=JsonParser.parseString(line).getAsJsonArray();Object result;
                BlockStateModelDispatcher decoded;
                try { decoded=BlockStateModelDispatcher.CODEC.parse(JsonOps.INSTANCE,StrictJsonParser.parse(row.get(1).getAsString())).getOrThrow(); }
                catch(RuntimeException error) {output.write(JSON.toJson(map("name",row.get(0).getAsString(),"error",true)));output.newLine();continue;}
                result=map("name",row.get(0).getAsString(),"definition",definition(decoded));
                output.write(JSON.toJson(result));output.newLine();
            }
        }
    }
}
