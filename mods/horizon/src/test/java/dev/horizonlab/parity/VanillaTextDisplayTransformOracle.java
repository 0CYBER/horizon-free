package dev.horizonlab.parity;

import com.google.gson.*;
import io.netty.buffer.Unpooled;
import java.nio.file.*;
import java.util.*;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.world.entity.Display;
import org.joml.*;

/** Invoke the real client renderer's private billboard method without opening a window. */
public final class VanillaTextDisplayTransformOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe"); unsafeField.setAccessible(true);
        Object unsafe = unsafeField.get(null);
        var allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        var level = (VanillaTextDisplayOracle.Level) allocate.invoke(unsafe, VanillaTextDisplayOracle.Level.class);
        Class<?> renderer = Class.forName("net.minecraft.client.renderer.entity.DisplayRenderer");
        Class<?> concrete = Class.forName("net.minecraft.client.renderer.entity.DisplayRenderer$TextDisplayRenderer");
        Class<?> renderState = Class.forName("net.minecraft.client.renderer.entity.state.DisplayEntityRenderState");
        Object instance = allocate.invoke(unsafe, concrete);
        var orientation = renderer.getDeclaredMethod("calculateOrientation", Display.RenderState.class, renderState, Quaternionf.class);
        orientation.setAccessible(true);
        var fresh = Display.class.getDeclaredMethod("createFreshRenderState"); fresh.setAccessible(true);
        var output = new ArrayList<String>();
        var type = net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getValue(net.minecraft.resources.Identifier.parse("minecraft:text_display"));
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            JsonObject input = JsonParser.parseString(line).getAsJsonObject();
            var display = new Display.TextDisplay(type, level);
            var buf = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(HexFormat.of().parseHex(input.get("wire").getAsString())), ParityData.access);
            display.getEntityData().assignValues(ClientboundSetEntityDataPacket.STREAM_CODEC.decode(buf).packedItems()); buf.release();
            Object state = Class.forName("net.minecraft.client.renderer.entity.state.TextDisplayEntityRenderState").getConstructor().newInstance();
            String[] fields = {"entityYRot", "entityXRot", "cameraYRot", "cameraXRot"};
            for (int i=0;i<4;i++) renderState.getField(fields[i]).setFloat(state,input.getAsJsonArray("angles").get(i).getAsFloat());
            var data = (Display.RenderState) fresh.invoke(display);
            var q = (Quaternionf) orientation.invoke(instance,data,state,new Quaternionf());
            float[] matrix = new Matrix4f().rotate(q).mul(data.transformation().get(1).getMatrix()).get(new float[16]);
            output.add(new Gson().toJson(matrix));
        }
        Files.write(Path.of(args[1]),output);
    }
}
