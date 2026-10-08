package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.levelgen.densityfunction.*;

/** Original transport using BiomeSource.CODEC and decoded vanilla resolvers. */
public final class VanillaBiomeSourceCodecOracle {
    private static JsonElement normalized(JsonElement value) {
        if(value.isJsonArray()){var a=new JsonArray();for(var v:value.getAsJsonArray())a.add(normalized(v));return a;}
        if(value.isJsonObject()){var a=new JsonObject();for(var key:new TreeSet<>(value.getAsJsonObject().keySet()))a.add(key,normalized(value.getAsJsonObject().get(key)));return a;}
        if(value.isJsonPrimitive()&&value.getAsJsonPrimitive().isNumber()){var a=new JsonObject();a.addProperty("float64_bits",String.format("%016x",Double.doubleToRawLongBits(Double.parseDouble(value.toString()))));return a;}
        return value;
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);
        var load=VanillaMaterialConditionOracle.class.getDeclaredMethod("loadBiomes",Path.class);load.setAccessible(true);load.invoke(null,dir);
        var field=VanillaMaterialConditionOracle.class.getDeclaredField("biomes");field.setAccessible(true);var biomes=(MappedRegistry<Biome>)field.get(null);
        var presets=new MappedRegistry<MultiNoiseBiomeSourceParameterList>(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST,Lifecycle.stable());
        for(var preset:List.of(MultiNoiseBiomeSourceParameterList.Preset.OVERWORLD,MultiNoiseBiomeSourceParameterList.Preset.NETHER))presets.register(ResourceKey.create(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST,preset.id()),new MultiNoiseBiomeSourceParameterList(preset,biomes),RegistrationInfo.BUILT_IN);presets.freeze();
        var ops=RegistryOps.create(JsonOps.INSTANCE,new RegistryOps.RegistryInfoLookup(){public <T> Optional<HolderGetter<T>> lookup(ResourceKey<? extends Registry<? extends T>> key){
            if(key.equals(Registries.BIOME))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(biomes);
            if(key.equals(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST))return (Optional<HolderGetter<T>>)(Optional<?>)Optional.of(presets);return Optional.empty();}});
        var positions=JsonParser.parseString(Files.readString(dir.resolve("positions.json"))).getAsJsonArray();
        var zero=new DensitySampler(){public float sampleValue(SamplerContext c,int x,int y,int z){return 0;}public void sampleVolume(SamplerContext c,DensityBuffer b,DensityVolume v){b.fill(0);}}.bind(SamplerContext.builder().enableCaches().build());
        var climate=new Climate.Sampler(zero,zero,zero,zero,zero,zero);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var json:Files.readAllLines(dir.resolve("inputs.txt"))) {
                Optional<BiomeSource> decoded;
                try { decoded=BiomeSource.CODEC.parse(ops,JsonParser.parseString(json)).result(); }catch(IllegalArgumentException|IllegalStateException error){decoded=Optional.empty();}
                if(decoded.isEmpty()){out.println("codec error");continue;}
                var source=decoded.get();out.println("codec "+normalized(BiomeSource.CODEC.encodeStart(ops,source).getOrThrow()));
                var names=new TreeSet<String>();for(var biome:source.possibleBiomes())names.add(biome.unwrapKey().orElseThrow().identifier().toString());out.println("possible "+new Gson().toJson(names));
                var resolver=source.createResolver(climate);
                for(var p:positions){var q=p.getAsJsonArray();try{out.println("biome "+resolver.getNoiseBiome(q.get(0).getAsInt(),q.get(1).getAsInt(),q.get(2).getAsInt()).unwrapKey().orElseThrow().identifier());}catch(ArithmeticException|IllegalStateException error){out.println("biome error");}}
            }
        }
    }
}
