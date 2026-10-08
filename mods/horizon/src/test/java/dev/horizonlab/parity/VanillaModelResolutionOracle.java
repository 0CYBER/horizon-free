package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.geometry.UnbakedGeometry;
import net.minecraft.client.resources.model.sprite.Material;
import net.minecraft.resources.Identifier;
import net.minecraft.world.item.ItemDisplayContext;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.*;

/** Exercises actual ModelDiscovery, ResolvedModel and TextureSlots resolution. */
public final class VanillaModelResolutionOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static Object material(Material value) {
        return value == null ? null : map("sprite", value.sprite().toString(), "translucent", value.forceTranslucent());
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        try (var input=Files.newBufferedReader(Path.of(args[0])); var output=Files.newBufferedWriter(Path.of(args[1]))) {
            int index=0;
            for (String line; (line=input.readLine())!=null; index++) {
                var row=JsonParser.parseString(line).getAsJsonObject();
                var models=new HashMap<Identifier, UnbakedModel>();
                var slots=new TreeSet<String>(); slots.add("missingno"); slots.add("particle");
                for (var entry:row.getAsJsonObject("models").entrySet()) {
                    var model=CuboidModel.fromStream(new StringReader(entry.getValue().getAsString()));
                    models.put(Identifier.parse(entry.getKey()),model);
                    slots.addAll(model.textureSlots().values().keySet());
                }
                var discovery=new ModelDiscovery(models,MissingCuboidModel.missingModel());
                discovery.addRoot(resolver -> {for(var root:row.getAsJsonArray("roots")) resolver.markDependency(Identifier.parse(root.getAsString()));});
                var resolved=discovery.resolve();
                for (var root:row.getAsJsonArray("roots")) {
                    String name=root.getAsString();
                    var model=resolved.get(Identifier.parse(name));
                    if(model==null) {output.write(JSON.toJson(map("case",index,"name",name,"error",true)));output.newLine();continue;}
                    var geometry=model.getTopGeometry();
                    var elements=new ArrayList<Object>();
                    if(geometry!=UnbakedGeometry.EMPTY) for(var e:((UnbakedCuboidGeometry)geometry).elements()) elements.add(element(e));
                    var display=new TreeMap<String,Object>();
                    var transforms=model.getTopTransforms();
                    for(var context:ItemDisplayContext.values()) if(context!=ItemDisplayContext.NONE) display.put(context.getSerializedName(),transform(transforms.getTransform(context)));
                    var textures=model.getTopTextureSlots();
                    var values=new TreeMap<String,Object>();
                    for(var slot:slots) {var value=textures.getMaterial("#"+slot);if(value!=null) values.put(slot,material(value));}
                    var queries=new ArrayList<Object>();
                    for(var raw:row.getAsJsonArray("queries")) {
                        String slot=raw.getAsString();
                        try {queries.add(map("slot",slot,"value",material(textures.getMaterial(slot))));}
                        catch(RuntimeException failure) {queries.add(map("slot",slot,"error",true));}
                    }
                    output.write(JSON.toJson(map("case",index,"name",name,"ao",model.getTopAmbientOcclusion(),
                        "gui",model.getTopGuiLight().name().toLowerCase(Locale.ROOT),"elements",elements,
                        "display",display,"textures",values,"queries",queries)));output.newLine();
                }
            }
        }
    }
}
