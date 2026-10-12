package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.audio.ListenerTransform;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.SubtitleOverlay;
import net.minecraft.client.resources.sounds.SimpleSoundInstance;
import net.minecraft.client.resources.sounds.SoundInstance;
import net.minecraft.client.sounds.SoundEventListener;
import net.minecraft.client.sounds.SoundManager;
import net.minecraft.client.sounds.WeighedSoundEvents;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.RandomSource;
import net.minecraft.util.Util;
import net.minecraft.world.phys.Vec3;
import org.joml.Matrix3x2f;
import org.joml.Matrix3x2fStack;

/**
 * Runs vanilla's own {@link SubtitleOverlay} on scripted cases: sounds the sound engine reports to its listeners
 * (subtitle, position, range, looping) at chosen times of {@code Util.getMillis}, and frames that render the
 * overlay for a listener transform, the subtitle options (shown or not, notification display time, text
 * background opacity and "chat only") and a GUI size. A sound reaches the overlay only while it listens, as with
 * the real {@code SoundEngine}; it starts and stops listening when a frame renders with the option on or off.
 * Every frame records each fill and text call with the pose translation, in order.
 *
 * <pre>  INPUTS OUTPUT</pre>
 */
public final class VanillaSubtitleOracle {
    static Object unsafe;
    static Method allocate;
    static long nanos;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws NoSuchFieldException {
        for (Class<?> k = type; k != null; k = k.getSuperclass()) {
            try {
                Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
            }
        }
        throw new NoSuchFieldException(name + " on " + type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = field(owner, name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat" : t == long.class ? "putLong" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, target, offset, value);
    }

    /** The sound manager's listener list and listener transform. */
    public static final class Sounds extends SoundManager {
        List<SoundEventListener> listeners;
        ListenerTransform transform;

        private Sounds() {
            super(null);
        }

        @Override
        public void addListener(final SoundEventListener listener) {
            this.listeners.add(listener);
        }

        @Override
        public void removeListener(final SoundEventListener listener) {
            this.listeners.remove(listener);
        }

        @Override
        public ListenerTransform getListenerTransform() {
            return this.transform;
        }
    }

    /** Records fill and text calls with the pose. */
    public static final class Capture extends GuiGraphicsExtractor {
        JsonArray draws;
        Matrix3x2fStack stack;
        int width, height;

        private Capture() {
            super(null, null, 0, 0);
        }

        @Override
        public int guiWidth() {
            return this.width;
        }

        @Override
        public int guiHeight() {
            return this.height;
        }

        @Override
        public Matrix3x2fStack pose() {
            return this.stack;
        }

        @Override
        public void nextStratum() {
            JsonObject o = new JsonObject();
            o.addProperty("type", "stratum");
            this.draws.add(o);
        }

        void pose(JsonObject o) {
            Matrix3x2f m = this.stack;
            JsonArray a = new JsonArray();
            for (float v : new float[] {m.m00, m.m01, m.m10, m.m11, m.m20, m.m21}) {
                a.add(v);
            }
            o.add("pose", a);
        }

        @Override
        public void fill(final int x0, final int y0, final int x1, final int y1, final int color) {
            JsonObject o = new JsonObject();
            o.addProperty("type", "fill");
            o.addProperty("x0", x0);
            o.addProperty("y0", y0);
            o.addProperty("x1", x1);
            o.addProperty("y1", y1);
            o.addProperty("color", Integer.toUnsignedString(color, 16));
            this.pose(o);
            this.draws.add(o);
        }

        @Override
        public void text(final Font font, final FormattedCharSequence str, final int x, final int y, final int color, final boolean shadow) {
            JsonObject o = new JsonObject();
            o.addProperty("type", "text");
            StringBuilder sb = new StringBuilder();
            str.accept((index, style, cp) -> {
                sb.appendCodePoint(cp);
                return true;
            });
            o.addProperty("text", sb.toString());
            o.addProperty("x", x);
            o.addProperty("y", y);
            o.addProperty("color", Integer.toUnsignedString(color, 16));
            o.addProperty("shadow", shadow);
            this.pose(o);
            this.draws.add(o);
        }
    }

    @SuppressWarnings("unchecked")
    static <T> void option(Options options, String name, T value) throws Exception {
        OptionInstance<T> instance = (OptionInstance<T>) alloc(OptionInstance.class);
        put(instance, OptionInstance.class, "value", value);
        put(options, Options.class, name, instance);
    }

    static Vec3 vec(JsonElement e) {
        JsonArray a = e.getAsJsonArray();
        return new Vec3(a.get(0).getAsDouble(), a.get(1).getAsDouble(), a.get(2).getAsDouble());
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        Font font = ParityFont.create(ParityFont.resources());
        Util.setTimeSource(() -> nanos);
        Gson gson = new GsonBuilder().disableHtmlEscaping().serializeNulls().create();
        StringBuilder out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]), StandardCharsets.UTF_8)) {
            if (line.isBlank()) {
                continue;
            }
            JsonObject c = JsonParser.parseString(line).getAsJsonObject();
            JsonObject result = new JsonObject();
            result.add("case", c.get("case"));
            JsonArray frames = new JsonArray();
            try {
                Minecraft mc = alloc(Minecraft.class);
                Sounds sounds = alloc(Sounds.class);
                sounds.listeners = new ArrayList<>();
                sounds.transform = ListenerTransform.INITIAL;
                put(mc, Minecraft.class, "soundManager", sounds);
                put(mc, Minecraft.class, "font", font);
                Options options = alloc(Options.class);
                put(mc, Minecraft.class, "options", options);
                SubtitleOverlay overlay = new SubtitleOverlay(mc);
                for (JsonElement e : c.getAsJsonArray("ops")) {
                    JsonObject op = e.getAsJsonObject();
                    switch (op.get("op").getAsString()) {
                        case "time" -> nanos = op.get("ms").getAsLong() * 1_000_000L + op.get("sub").getAsLong();
                        case "play" -> {
                            Vec3 at = vec(op.get("at"));
                            JsonElement key = op.get("key");
                            WeighedSoundEvents event = new WeighedSoundEvents(Identifier.withDefaultNamespace("test"), key.isJsonNull() ? null : key.getAsString());
                            SoundInstance sound = new SimpleSoundInstance(Identifier.withDefaultNamespace("test"), SoundSource.MASTER, 1.0F, 1.0F,
                                RandomSource.create(0L), op.get("looping").getAsBoolean(), 0, SoundInstance.Attenuation.LINEAR, at.x, at.y, at.z, false);
                            float range = op.get("range").isJsonNull() ? Float.POSITIVE_INFINITY : op.get("range").getAsFloat();
                            for (SoundEventListener listener : List.copyOf(sounds.listeners)) {
                                listener.onPlaySound(sound, event, range);
                            }
                        }
                        case "frame" -> {
                            sounds.transform = new ListenerTransform(vec(op.get("pos")), vec(op.get("forward")), vec(op.get("up")));
                            option(options, "showSubtitles", op.get("show").getAsBoolean());
                            option(options, "notificationDisplayTime", op.get("mult").getAsDouble());
                            option(options, "textBackgroundOpacity", op.get("opacity").getAsDouble());
                            option(options, "backgroundForChatOnly", op.get("chat_only").getAsBoolean());
                            Capture g = alloc(Capture.class);
                            g.draws = new JsonArray();
                            g.stack = new Matrix3x2fStack(16);
                            g.width = op.get("w").getAsInt();
                            g.height = op.get("h").getAsInt();
                            overlay.extractRenderState(g);
                            frames.add(g.draws);
                        }
                        default -> throw new IllegalArgumentException(op.toString());
                    }
                }
                result.add("frames", frames);
            } catch (Throwable t) {
                result.addProperty("error", t.toString());
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
