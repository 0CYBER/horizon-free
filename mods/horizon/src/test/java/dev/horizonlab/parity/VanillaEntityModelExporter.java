package dev.horizonlab.parity;

import com.google.gson.*;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.client.model.geom.LayerDefinitions;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.server.Bootstrap;

/**
 * Original exporter of every baked entity model layer (part tree, initial poses and
 * textured polygons) produced by vanilla's own LayerDefinitions.
 */
public final class VanillaEntityModelExporter {
    private static Field cubes;
    private static Field children;

    @SuppressWarnings("unchecked")
    private static JsonObject part(ModelPart part) throws Exception {
        var o = new JsonObject();
        var pose = new JsonArray();
        for (float f : new float[] {part.x, part.y, part.z, part.xRot, part.yRot, part.zRot, part.xScale, part.yScale, part.zScale})
            pose.add(f);
        o.add("pose", pose);
        var polygons = new JsonArray();
        for (ModelPart.Cube cube : (List<ModelPart.Cube>) cubes.get(part)) {
            for (ModelPart.Polygon polygon : cube.polygons) {
                var p = new JsonArray();
                for (ModelPart.Vertex v : polygon.vertices()) {
                    var vertex = new JsonArray();
                    vertex.add(v.x());
                    vertex.add(v.y());
                    vertex.add(v.z());
                    vertex.add(v.u());
                    vertex.add(v.v());
                    p.add(vertex);
                }
                polygons.add(p);
            }
        }
        o.add("polygons", polygons);
        var kids = new JsonObject();
        for (var entry : ((Map<String, ModelPart>) children.get(part)).entrySet())
            kids.add(entry.getKey(), part(entry.getValue()));
        o.add("children", kids);
        return o;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        cubes = ModelPart.class.getDeclaredField("cubes");
        cubes.setAccessible(true);
        children = ModelPart.class.getDeclaredField("children");
        children.setAccessible(true);
        var out = new JsonObject();
        for (var entry : LayerDefinitions.createRoots().entrySet()) {
            try {
                out.add(entry.getKey().toString(), part(entry.getValue().bakeRoot()));
            } catch (RuntimeException error) {
                System.err.println("skipped " + entry.getKey() + ": " + error);
            }
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(out), StandardCharsets.UTF_8);
    }
}
