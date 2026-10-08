package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.world.level.biome.Climate;
import net.minecraft.world.level.biome.MultiNoiseBiomeSourceParameterList;

/** Original private-data transport; vanilla builds the preset parameter lists. */
public final class VanillaBiomeExport {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var result=new JsonObject();
        var presets=new java.util.TreeMap<String,Climate.ParameterList<net.minecraft.resources.ResourceKey<net.minecraft.world.level.biome.Biome>>>();
        MultiNoiseBiomeSourceParameterList.knownPresets().forEach((preset,list)->presets.put(preset.id().toString(),list));
        for(var preset:presets.entrySet()) {
            var entries=new JsonArray();
            for(var value:preset.getValue().values()) {
                var point=value.getFirst();var entry=new JsonObject();var coordinates=new JsonArray();
                for(var parameter:new Climate.Parameter[]{point.temperature(),point.humidity(),point.continentalness(),point.erosion(),point.depth(),point.weirdness()}) {
                    var interval=new JsonArray();interval.add(parameter.min());interval.add(parameter.max());coordinates.add(interval);
                }
                entry.add("coordinates",coordinates);entry.addProperty("offset",point.offset());entry.addProperty("biome",value.getSecond().identifier().toString());entries.add(entry);
            }
            result.addProperty(preset.getKey(),entries.toString());
        }
        Files.writeString(Path.of(args[0]),result.toString(),StandardCharsets.UTF_8);
    }
}
