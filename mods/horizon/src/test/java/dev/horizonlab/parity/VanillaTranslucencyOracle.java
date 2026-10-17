package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.vertex.*;
import com.mojang.renderpearl.api.pipeline.IndexType;
import java.nio.*;
import java.nio.file.*;
import net.minecraft.client.renderer.chunk.TranslucencyPointOfView;
import net.minecraft.core.BlockPos;
import net.minecraft.core.SectionPos;
import net.minecraft.world.phys.Vec3;

/** Original probe calling the pinned game's centroid, sorting and index writers. */
public final class VanillaTranslucencyOracle {
    public static void main(String[] args) throws Exception {
        try (var out = Files.newBufferedWriter(Path.of(args[1]))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                JsonObject input = JsonParser.parseString(line).getAsJsonObject();
                JsonArray quads = input.getAsJsonArray("quads");
                ByteBuffer vertices = ByteBuffer.allocate(quads.size() * 48).order(ByteOrder.nativeOrder());
                for (JsonElement quad : quads)
                    for (JsonElement vertex : quad.getAsJsonArray())
                        for (JsonElement component : vertex.getAsJsonArray()) vertices.putFloat(component.getAsFloat());
                vertices.flip();
                CompactVectorArray centers = new CompactVectorArray(quads.size());
                MeshData.decodeQuadCentroids(vertices, quads.size() * 4, DefaultVertexFormat.POSITION, centers, 0);
                JsonArray centerBits = new JsonArray();
                for (int i = 0; i < centers.size(); i++) {
                    JsonArray c = new JsonArray();
                    c.add(Float.floatToIntBits(centers.getX(i)));
                    c.add(Float.floatToIntBits(centers.getY(i)));
                    c.add(Float.floatToIntBits(centers.getZ(i)));
                    centerBits.add(c);
                }
                JsonArray origin = input.getAsJsonArray("origin");
                int ox = origin.get(0).getAsInt(), oy = origin.get(1).getAsInt(), oz = origin.get(2).getAsInt();
                long section = SectionPos.asLong(ox >> 4, oy >> 4, oz >> 4);
                BlockPos lastBlock = null;
                TranslucencyPointOfView lastPov = null;
                JsonArray frames = new JsonArray();
                for (JsonElement element : input.getAsJsonArray("cameras")) {
                    JsonArray c = element.getAsJsonArray();
                    Vec3 camera = new Vec3(c.get(0).getAsDouble(), c.get(1).getAsDouble(), c.get(2).getAsDouble());
                    BlockPos block = BlockPos.containing(camera);
                    TranslucencyPointOfView pov = TranslucencyPointOfView.of(camera, section);
                    boolean update = lastPov == null || !lastPov.equals(pov) || (!block.equals(lastBlock) && pov.isAxisAligned());
                    if (update) { lastPov = pov; lastBlock = block; }
                    var sorting = VertexSorting.byDistance((float)(camera.x - ox), (float)(camera.y - oy), (float)(camera.z - oz));
                    ByteBuffer buffer = ByteBuffer.allocate(quads.size() * 24).order(ByteOrder.nativeOrder());
                    new MeshData.SortState(centers, IndexType.INT).writeSortedIndexBuffer(buffer, sorting);
                    buffer.flip();
                    JsonArray indices = new JsonArray();
                    while (buffer.hasRemaining()) indices.add(buffer.getInt());
                    JsonObject frame = new JsonObject();
                    frame.add("indices", indices); frame.addProperty("update", update); frames.add(frame);
                }
                JsonObject result = new JsonObject();
                result.add("centers", centerBits); result.add("frames", frames);
                out.write(result.toString()); out.newLine();
            }
        }
    }
}
