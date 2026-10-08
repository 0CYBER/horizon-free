package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.sprite.TextureSlots;
import net.minecraft.world.item.ItemDisplayContext;
import org.joml.Vector3fc;

/** Original adapter around the pinned client parser. No model assets are embedded. */
public final class VanillaCuboidModelOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static long bits(float value) { return Integer.toUnsignedLong(Float.floatToIntBits(value)); }
    private static Object vector(Vector3fc value) { return new long[]{bits(value.x()), bits(value.y()), bits(value.z())}; }
    private static String direction(net.minecraft.core.Direction value) { return value == null ? null : value.getName(); }
    static Map<String, Object> map(Object... values) {
        var result = new TreeMap<String, Object>();
        for (int i=0;i<values.length;i+=2) result.put((String)values[i], values[i+1]);
        return result;
    }
    static Object transform(ItemTransform value) {
        if (value == ItemTransform.NO_TRANSFORM) return "inherit";
        if (value == null) return null;
        return map("rotation", vector(value.rotation()), "translation", vector(value.translation()), "scale", vector(value.scale()));
    }
    private static Object rotation(CuboidRotation value) {
        if (value == null) return null;
        Object angles = switch (value.value()) {
            case CuboidRotation.SingleAxisRotation single -> map("axis", single.axis().getName(), "angle", bits(single.angle()));
            case CuboidRotation.EulerXYZRotation euler -> new long[]{bits(euler.x()), bits(euler.y()), bits(euler.z())};
            default -> throw new AssertionError("Unexpected parsed rotation " + value.value().getClass());
        };
        return map("origin", vector(value.origin()), "value", angles, "rescale", value.rescale());
    }
    private static Object face(CuboidFace value) {
        if (value == null) return null;
        var uv = value.uvs();
        return map("cull", direction(value.cullForDirection()), "tint", value.tintIndex(), "texture", value.texture(),
            "rotation", value.rotation().ordinal(), "uv", uv == null ? null : new long[]{bits(uv.minU()), bits(uv.minV()), bits(uv.maxU()), bits(uv.maxV())});
    }
    static Object element(CuboidModelElement value) {
        if (value == null) return null;
        var faces = new TreeMap<String, Object>();
        value.faces().forEach((key, f) -> faces.put(key.getName(), face(f)));
        return map("from", vector(value.from()), "to", vector(value.to()), "faces", faces,
            "rotation", rotation(value.rotation()), "shade", direction(value.shadeDirectionOverride()), "light", value.lightEmission());
    }
    private static Object model(CuboidModel value) throws Exception {
        if (value == null) return null;
        var textures = new TreeMap<String, Object>();
        // Slot implementation records are package-private; inspect only their public accessors.
        for (var entry : value.textureSlots().values().entrySet()) {
            Object slot = entry.getValue();
            var cls = slot.getClass();
            if (cls.getSimpleName().equals("Reference")) {
                var target = cls.getDeclaredMethod("target"); target.setAccessible(true);
                textures.put(entry.getKey(), map("reference", target.invoke(slot)));
            } else {
                var method = cls.getDeclaredMethod("material"); method.setAccessible(true);
                var material = (net.minecraft.client.resources.model.sprite.Material)method.invoke(slot);
                textures.put(entry.getKey(), map("sprite", material.sprite().toString(), "translucent", material.forceTranslucent()));
            }
        }
        List<Object> elements = null;
        if (value.geometry() != null) {
            elements = new ArrayList<>();
            for (var element : ((UnbakedCuboidGeometry)value.geometry()).elements()) elements.add(element(element));
        }
        Object display = null;
        if (value.transforms() != null) {
            var transforms = new TreeMap<String, Object>();
            for (var context : ItemDisplayContext.values()) {
                if (context != ItemDisplayContext.NONE) transforms.put(context.getSerializedName(), transform(value.transforms().getTransform(context)));
            }
            display = transforms;
        }
        return map("parent", value.parent() == null ? null : value.parent().toString(), "ao", value.ambientOcclusion(),
            "gui", value.guiLight() == null ? null : value.guiLight().name().toLowerCase(Locale.ROOT), "textures", textures, "elements", elements, "display", display);
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        try (var input=Files.newBufferedReader(Path.of(args[0])); var output=Files.newBufferedWriter(Path.of(args[1]))) {
            for (String line; (line=input.readLine())!=null;) {
                var row=JsonParser.parseString(line).getAsJsonArray();
                CuboidModel decoded;
                try { decoded=CuboidModel.fromStream(new StringReader(row.get(1).getAsString())); }
                catch (RuntimeException failure) {
                    output.write(JSON.toJson(map("name", row.get(0).getAsString(), "error", true))); output.newLine(); continue;
                }
                output.write(JSON.toJson(map("name", row.get(0).getAsString(), "model", model(decoded)))); output.newLine();
            }
        }
    }
}
