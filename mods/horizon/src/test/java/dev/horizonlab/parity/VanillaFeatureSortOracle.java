package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.level.biome.FeatureSorter;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.PlacedFeature;

/** Exercises identity-based native feature ordering, using equal dummy payloads. */
public final class VanillaFeatureSortOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var noop=Feature.DIRECT_CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString("{\"type\":\"no_op\"}")).getOrThrow();
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8));var inputs=Files.lines(dir.resolve("inputs.txt"))) {
            for(var line:(Iterable<String>)inputs::iterator) {
                var f=JsonParser.parseString(line).getAsJsonObject();
                var identities=new HashMap<Integer,PlacedFeature>();var names=new IdentityHashMap<PlacedFeature,Integer>();
                java.util.function.IntFunction<PlacedFeature> feature=id->identities.computeIfAbsent(id,key->{var value=new PlacedFeature(Holder.direct(noop),List.of());names.put(value,key);return value;});
                var biomes=new ArrayList<List<HolderSet<PlacedFeature>>>();
                for(var b:f.getAsJsonArray("biomes")) {
                    var steps=new ArrayList<HolderSet<PlacedFeature>>();
                    for(var s:b.getAsJsonArray()) {
                        var values=new ArrayList<Holder<PlacedFeature>>();for(var id:s.getAsJsonArray())values.add(Holder.direct(feature.apply(id.getAsInt())));
                        steps.add(HolderSet.direct(values));
                    }
                    biomes.add(steps);
                }
                try {
                    var steps=FeatureSorter.buildFeaturesPerStep(biomes,value->value,false);
                    out.println("steps "+steps.size());
                    for(int i=0;i<steps.size();i++) {
                        var step=steps.get(i);out.println("step "+i+" "+step.features().size());
                        for(var value:step.features())out.println("feature "+names.get(value));
                        for(var id:f.getAsJsonArray("queries"))out.println("index "+id.getAsInt()+" "+step.indexMapping().applyAsInt(feature.apply(id.getAsInt())));
                    }
                } catch(IllegalStateException error){out.println("cycle");}
            }
        }
    }
}
