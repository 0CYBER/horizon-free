package dev.horizonlab.parity;

import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.renderer.CloudRenderer;

/** Invoke the shipped mesh builder without constructing GPU resources. */
public final class VanillaCloudRenderOracle {
    public static void main(String[] args) throws Exception {
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        var field = unsafeClass.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        Object renderer = unsafeClass.getMethod("allocateInstance", Class.class)
            .invoke(field.get(null), CloudRenderer.class);
        Class<?> relative = Class.forName("net.minecraft.client.renderer.CloudRenderer$RelativeCameraPos");
        Object[] positions = relative.getEnumConstants(); // ABOVE, INSIDE, BELOW
        var cell = CloudRenderer.class.getDeclaredMethod("buildExtrudedCell", relative, ByteBuffer.class, int.class, int.class, long.class);
        var mesh = CloudRenderer.class.getDeclaredMethod("buildMesh", relative, ByteBuffer.class, int.class, int.class, boolean.class, int.class);
        cell.setAccessible(true);
        mesh.setAccessible(true);
        var texture = CloudRenderer.class.getDeclaredField("texture");
        texture.setAccessible(true);
        texture.set(renderer, new CloudRenderer.TextureData(new long[]{0xfffffff0L}, 1, 1));
        StringBuilder output = new StringBuilder();
        int index = 0;
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            String[] parts = line.split(" ");
            int kind = Integer.parseInt(parts[0]), x = Integer.parseInt(parts[1]), z = Integer.parseInt(parts[2]);
            int bottom = Integer.parseInt(parts[3]), neighbors = Integer.parseInt(parts[4]);
            Object pos = positions[bottom > 0 ? 2 : bottom + 4 < 0 ? 0 : 1];
            ByteBuffer buffer = ByteBuffer.allocate(kind == 2 ? 4_000_000 : 100000);
            if (kind == 0) {
                long flags = ((neighbors & 1) != 0 ? 8 : 0) | ((neighbors & 2) != 0 ? 2 : 0)
                    | ((neighbors & 4) != 0 ? 1 : 0) | ((neighbors & 8) != 0 ? 4 : 0);
                cell.invoke(renderer, pos, buffer, x, z, 0xfffffff0L | flags);
            } else if (kind == 1) {
                mesh.invoke(renderer, pos, buffer, 0, 0, true, x);
            } else {
                // prepare(..., range, ...): radiusCells from the Cloud Distance option.
                int radiusBlocks = x * 16;
                mesh.invoke(renderer, pos, buffer, 0, 0, true, net.minecraft.util.Mth.ceil(radiusBlocks / 12.0F));
            }
            output.append(index++);
            buffer.flip();
            while (buffer.hasRemaining()) {
                int cx = buffer.get(), cz = buffer.get(), flags = Byte.toUnsignedInt(buffer.get());
                cx = (cx << 1) | ((flags & 128) >> 7);
                cz = (cz << 1) | ((flags & 64) >> 6);
                output.append(" ").append(cx).append(",").append(cz).append(",").append(flags & 7).append(",").append((flags & 16) == 0 ? 0 : 1);
            }
            output.append("\n");
        }
        Files.writeString(Path.of(args[1]), output);
    }
}
