package dev.horizonlab.parity;

import com.mojang.blaze3d.vertex.VertexConsumer;
import java.lang.reflect.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import net.minecraft.client.renderer.WeatherEffectRenderer;
import net.minecraft.util.RandomSource;
import net.minecraft.world.phys.Vec3;

/** Build rain and snow column quads with the shipped renderer, without a GPU or a level. */
public final class VanillaWeatherRenderOracle {
    public static void main(String[] args) throws Exception {
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        var theUnsafe = unsafeClass.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        Object renderer = unsafeClass.getMethod("allocateInstance", Class.class)
            .invoke(theUnsafe.get(null), WeatherEffectRenderer.class);
        // The constructor's column table, built the same way before it reaches Minecraft.getInstance().
        float[] sizeX = new float[1024], sizeZ = new float[1024];
        for (int z = 0; z < 32; z++) {
            for (int x = 0; x < 32; x++) {
                float deltaX = x - 16, deltaZ = z - 16;
                float distance = net.minecraft.util.Mth.length(deltaX, deltaZ);
                sizeX[z * 32 + x] = -deltaZ / distance;
                sizeZ[z * 32 + x] = deltaX / distance;
            }
        }
        for (String name : new String[]{"columnSizeX", "columnSizeZ"}) {
            var field = WeatherEffectRenderer.class.getDeclaredField(name);
            field.setAccessible(true);
            field.set(renderer, name.endsWith("X") ? sizeX : sizeZ);
        }
        var rain = WeatherEffectRenderer.class.getDeclaredMethod("createRainColumnInstance",
            RandomSource.class, long.class, int.class, int.class, int.class, int.class, int.class, float.class);
        var snow = WeatherEffectRenderer.class.getDeclaredMethod("createSnowColumnInstance",
            RandomSource.class, long.class, int.class, int.class, int.class, int.class, int.class, float.class);
        var prepare = WeatherEffectRenderer.class.getDeclaredMethod("prepareInstances",
            VertexConsumer.class, List.class, Vec3.class, float.class, int.class, float.class);
        rain.setAccessible(true);
        snow.setAccessible(true);
        prepare.setAccessible(true);
        StringBuilder output = new StringBuilder();
        int index = 0;
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            String[] p = line.split(" ");
            int kind = Integer.parseInt(p[0]);
            long ticks = Long.parseLong(p[1]);
            int x = Integer.parseInt(p[2]), bottom = Integer.parseInt(p[3]), top = Integer.parseInt(p[4]);
            int z = Integer.parseInt(p[5]), light = Integer.parseInt(p[6]);
            float partial = Float.parseFloat(p[7]);
            Vec3 camera = new Vec3(Double.parseDouble(p[8]), Double.parseDouble(p[9]), Double.parseDouble(p[10]));
            int radius = Integer.parseInt(p[11]);
            float intensity = Float.parseFloat(p[12]);
            // extractRenderState: the column random is reseeded per column.
            RandomSource random = RandomSource.createThreadLocalInstance(0L);
            random.setSeed(x * x * 3121 + x * 45238971 ^ z * z * 418711 + z * 13761);
            Object column = (kind == 0 ? rain : snow).invoke(renderer, random, ticks, x, bottom, top, z, light, partial);
            var record = (WeatherEffectRenderer.ColumnInstance) column;
            output.append(index++).append(' ').append(record.x()).append(' ').append(record.z()).append(' ')
                .append(record.bottomY()).append(' ').append(record.topY()).append(' ')
                .append(bits(record.uOffset())).append(' ').append(bits(record.vOffset())).append(' ')
                .append(record.lightCoords());
            StringBuilder vertices = new StringBuilder();
            VertexConsumer consumer = (VertexConsumer) Proxy.newProxyInstance(
                VertexConsumer.class.getClassLoader(), new Class<?>[]{VertexConsumer.class}, (proxy, method, a) -> {
                    switch (method.getName()) {
                        case "addVertex" -> vertices.append(" |").append(bits((Float) a[0])).append(',')
                            .append(bits((Float) a[1])).append(',').append(bits((Float) a[2]));
                        case "setUv" -> vertices.append(',').append(bits((Float) a[0])).append(',').append(bits((Float) a[1]));
                        case "setColor" -> vertices.append(',').append(Integer.toHexString((Integer) a[0]));
                        case "setLight" -> vertices.append(',').append((Integer) a[0]);
                        default -> throw new IllegalStateException("unexpected " + method);
                    }
                    return proxy;
                });
            prepare.invoke(renderer, consumer, List.of(column), camera, kind == 0 ? 1.0F : 0.8F, radius, intensity);
            output.append(vertices).append('\n');
        }
        Files.writeString(Path.of(args[1]), output);
    }

    private static String bits(float value) {
        return Float.isNaN(value) ? "nan" : Integer.toHexString(Float.floatToRawIntBits(value));
    }
}
