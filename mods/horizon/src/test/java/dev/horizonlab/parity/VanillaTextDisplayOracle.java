package dev.horizonlab.parity;

import com.google.gson.*;
import io.netty.buffer.Unpooled;
import java.nio.file.*;
import java.lang.reflect.*;
import java.util.*;
import net.minecraft.nbt.TagParser;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.*;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.world.entity.*;
import org.joml.*;

/** Actual accessors/defaults and wire bytes, not a second implementation of their layout. */
public final class VanillaTextDisplayOracle {
    static final class Level extends net.minecraft.server.level.ServerLevel {
        Level() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public int getNextEntityId() { return 7; }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return ParityData.access; }
    }
    static final String[] KEYS = {"interpolation_delay", "interpolation_duration", "teleport_duration",
        "translation", "scale", "left_rotation", "right_rotation", "billboard", "brightness", "view_range",
        "shadow_radius", "shadow_strength", "width", "height", "glow_color", "text", "line_width", "background", "opacity", "flags"};
    static final String[] FIELDS = {"DATA_TRANSFORMATION_INTERPOLATION_START_DELTA_TICKS_ID", "DATA_TRANSFORMATION_INTERPOLATION_DURATION_ID", "DATA_POS_ROT_INTERPOLATION_DURATION_ID",
        "DATA_TRANSLATION_ID", "DATA_SCALE_ID", "DATA_LEFT_ROTATION_ID", "DATA_RIGHT_ROTATION_ID", "DATA_BILLBOARD_RENDER_CONSTRAINTS_ID", "DATA_BRIGHTNESS_OVERRIDE_ID", "DATA_VIEW_RANGE_ID",
        "DATA_SHADOW_RADIUS_ID", "DATA_SHADOW_STRENGTH_ID", "DATA_WIDTH_ID", "DATA_HEIGHT_ID", "DATA_GLOW_COLOR_OVERRIDE_ID", "DATA_TEXT_ID", "DATA_LINE_WIDTH_ID", "DATA_BACKGROUND_COLOR_ID", "DATA_TEXT_OPACITY_ID", "DATA_STYLE_FLAGS_ID"};
    static final Gson GSON = new Gson();

    @SuppressWarnings({"rawtypes", "unchecked"})
    public static void main(String[] args) throws Exception {
        ParityData.init();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        Object unsafe = unsafeField.get(null);
        var level = (Level) unsafe.getClass().getMethod("allocateInstance", Class.class).invoke(unsafe, Level.class);
        var out = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            JsonObject input = JsonParser.parseString(line).getAsJsonObject();
            var type = net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getValue(net.minecraft.resources.Identifier.parse("minecraft:text_display"));
            var display = new Display.TextDisplay(type, level);
            var state = new JsonObject();
            for (int i = 0; i < KEYS.length; i++) {
                Field field = (i < 15 ? Display.class : Display.TextDisplay.class).getDeclaredField(FIELDS[i]);
                field.setAccessible(true);
                EntityDataAccessor accessor = (EntityDataAccessor) field.get(null);
                Object value = display.getEntityData().get(accessor);
                if (input.has(KEYS[i])) {
                    JsonElement in = input.get(KEYS[i]);
                    if (value instanceof Integer) value = in.getAsInt();
                    else if (value instanceof Float) value = in.getAsFloat();
                    else if (value instanceof Byte) value = in.getAsByte();
                    else if (value instanceof Vector3f) value = new Vector3f(in.getAsJsonArray().get(0).getAsFloat(), in.getAsJsonArray().get(1).getAsFloat(), in.getAsJsonArray().get(2).getAsFloat());
                    else if (value instanceof Quaternionf) value = new Quaternionf(in.getAsJsonArray().get(0).getAsFloat(), in.getAsJsonArray().get(1).getAsFloat(), in.getAsJsonArray().get(2).getAsFloat(), in.getAsJsonArray().get(3).getAsFloat());
                    else if (value instanceof Component) value = ComponentSerialization.CODEC.parse(ParityData.ops, TagParser.parseCompoundFully("{v:" + in + "}").get("v")).getOrThrow();
                    display.getEntityData().set(accessor, value);
                }
                JsonElement encoded;
                if (value instanceof Vector3f v) encoded = GSON.toJsonTree(new float[]{v.x, v.y, v.z});
                else if (value instanceof Quaternionf q) encoded = GSON.toJsonTree(new float[]{q.x, q.y, q.z, q.w});
                else if (value instanceof Component c) encoded = new JsonPrimitive(c.getString());
                else encoded = GSON.toJsonTree(value);
                state.add(KEYS[i], encoded);
            }
            state.addProperty("alignment", Display.TextDisplay.getAlign(state.get("flags").getAsByte()).name());
            var values = display.getEntityData().getNonDefaultValues();
            var buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), ParityData.access);
            ClientboundSetEntityDataPacket.STREAM_CODEC.encode(buf, new ClientboundSetEntityDataPacket(7, values == null ? List.of() : values));
            byte[] bytes = new byte[buf.readableBytes()]; buf.readBytes(bytes); buf.release();
            var result = new JsonObject(); result.add("expected", state); result.addProperty("wire", HexFormat.of().formatHex(bytes));
            out.add(GSON.toJson(result));
        }
        Files.write(Path.of(args[1]), out);
    }
}
