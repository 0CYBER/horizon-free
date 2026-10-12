package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.commands.CommandEncoder;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.device.GpuDevice;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import java.lang.reflect.*;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.WorldBorderRenderer;
import net.minecraft.client.renderer.state.level.WorldBorderRenderState;
import net.minecraft.client.renderer.texture.AbstractTexture;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.Util;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.phys.Vec3;

/**
 * Original oracle for the world border as the client shows it: drives a
 * vanilla WorldBorder through the case's size changes and ticks, then runs
 * WorldBorderRenderer.extract, prepare (the wall vertex buffer, captured from
 * a stand-in GPU device), prepareDynamicTransforms (captured from a
 * stand-in DynamicGpuData, at a fixed Util.getMillis) and draw (the walls
 * drawn, from a stand-in RenderPass), and Hud.extractVignette (the vignette
 * colour) for every frame.
 *
 * Input lines: case JSON. Output lines: {case, frames:[...]} or {case, error}.
 */
public final class VanillaWorldBorderRenderOracle {
    static Object unsafe;
    static Method allocate;
    static float[] lastVertices;
    static boolean rebuilt;
    static Object lastTransform;
    static List<Integer> sides = new ArrayList<>();
    static int vignetteColor;
    static long millis;

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
        boolean isStatic = Modifier.isStatic(f.getModifiers());
        long offset = isStatic
            ? (long) unsafe.getClass().getMethod("staticFieldOffset", Field.class).invoke(unsafe, f)
            : (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        Object base = isStatic ? unsafe.getClass().getMethod("staticFieldBase", Field.class).invoke(unsafe, f) : target;
        Class<?> t = f.getType();
        String setter = t == int.class ? "putInt" : t == boolean.class ? "putBoolean" : t == float.class ? "putFloat"
            : t == long.class ? "putLong" : t == double.class ? "putDouble" : "putObject";
        Class<?> param = t.isPrimitive() ? t : Object.class;
        unsafe.getClass().getMethod(setter, Object.class, long.class, param).invoke(unsafe, base, offset, value);
    }

    static Object proxy(Class<?> type, InvocationHandler handler) {
        return Proxy.newProxyInstance(type.getClassLoader(), new Class<?>[] {type}, handler);
    }

    static Object defaults(Object proxy, Method method, Object[] args) throws Throwable {
        if (method.isDefault()) return InvocationHandler.invokeDefault(proxy, method, args);
        Class<?> r = method.getReturnType();
        if (r == boolean.class) return false;
        if (r == int.class || r == long.class || r == short.class || r == byte.class) return r == long.class ? (Object) 0L : (Object) 0;
        if (r == float.class) return 0f;
        if (r == double.class) return 0.0;
        return null;
    }

    /** Records the dynamic transform the renderer writes. */
    public static final class Uniforms extends DynamicGpuData {
        private Uniforms() { super(); }
        @Override public com.mojang.renderpearl.api.buffers.GpuBufferSlice writeTransform(final DynamicGpuData.Transform transform) {
            lastTransform = transform;
            return null;
        }
    }

    /** Records the vignette blit colour. */
    public static final class Capture extends GuiGraphicsExtractor {
        private Capture() { super(null, null, 0, 0); }
        @Override public int guiWidth() { return 427; }
        @Override public int guiHeight() { return 240; }
        @Override public void blit(RenderPipeline p, Identifier texture, int x, int y, float u, float v, int w, int h, int sw, int sh, int tw, int th, int color) {
            vignetteColor = color;
        }
    }

    static JsonArray floats(float... values) {
        var a = new JsonArray();
        for (float v : values) a.add(Float.floatToRawIntBits(v));
        return a;
    }

    static JsonArray doubles(double... values) {
        var a = new JsonArray();
        for (double v : values) a.add(Double.doubleToRawLongBits(v));
        return a;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        RenderSystem.initRenderThread();
        Util.setTimeSource(() -> millis * 1_000_000L);
        CommandEncoder encoder = (CommandEncoder) proxy(CommandEncoder.class, (p, m, a) -> {
            if (m.getName().equals("writeToBuffer")) {
                ByteBuffer data = ((ByteBuffer) a[1]).duplicate().order(ByteOrder.nativeOrder());
                float[] v = new float[data.remaining() / 4];
                for (int n = 0; n < v.length; n++) v[n] = data.getFloat(data.position() + n * 4);
                lastVertices = v;
                rebuilt = true;
                return null;
            }
            return defaults(p, m, a);
        });
        GpuDevice device = (GpuDevice) proxy(GpuDevice.class, (p, m, a) -> m.getName().equals("createCommandEncoder") ? encoder : defaults(p, m, a));
        put(null, RenderSystem.class, "DEVICE", device);
        put(null, RenderSystem.class, "dynamicGpuData", alloc(Uniforms.class));
        GpuBuffer buffer = (GpuBuffer) proxy(GpuBuffer.class, (p, m, a) -> m.getName().equals("size") ? (Object) 1024L : defaults(p, m, a));
        RenderPass pass = (RenderPass) proxy(RenderPass.class, (p, m, a) -> {
            if (m.getName().equals("drawMultipleIndexed")) {
                for (Object d : (Collection<?>) a[0]) sides.add(((RenderPass.Draw<?>) d).firstIndex() / 6);
                return null;
            }
            return defaults(p, m, a);
        });
        Method prepareTransforms = WorldBorderRenderer.class.getDeclaredMethod("prepareDynamicTransforms", WorldBorderRenderState.class, Vec3.class);
        Method draw = WorldBorderRenderer.class.getDeclaredMethod("draw", List.class, double.class, double.class, double.class, GpuBuffer.class, RenderPass.class);
        Method vignette = Hud.class.getDeclaredMethod("extractVignette", GuiGraphicsExtractor.class, Entity.class);
        for (Method m : new Method[] {prepareTransforms, draw, vignette}) m.setAccessible(true);
        Class<?> indicesType = Class.forName("com.mojang.blaze3d.systems.RenderSystem$AutoStorageIndexBuffer");
        var gson = new Gson();
        var out = new StringBuilder();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.isBlank()) continue;
            JsonObject c = JsonParser.parseString(line).getAsJsonObject();
            var result = new JsonObject();
            result.add("case", c);
            try {
                var border = new WorldBorder();
                border.setAbsoluteMaxSize(c.get("absoluteMaxSize").getAsInt());
                border.setCenter(c.get("centerX").getAsDouble(), c.get("centerZ").getAsDouble());
                border.setWarningBlocks(c.get("warningBlocks").getAsInt());
                border.setWarningTime(c.get("warningTime").getAsInt());
                border.setSize(c.get("size").getAsDouble());
                var renderer = alloc(WorldBorderRenderer.class);
                put(renderer, WorldBorderRenderer.class, "needsRebuild", true);
                put(renderer, WorldBorderRenderer.class, "worldBorderBuffer", buffer);
                put(renderer, WorldBorderRenderer.class, "indices", alloc(indicesType));
                var textures = alloc(TextureManager.class);
                var byPath = new HashMap<Identifier, AbstractTexture>();
                byPath.put(WorldBorderRenderer.FORCEFIELD_LOCATION, alloc(net.minecraft.client.renderer.texture.SimpleTexture.class));
                put(textures, TextureManager.class, "byPath", byPath);
                put(renderer, WorldBorderRenderer.class, "textureManager", textures);
                var state = new WorldBorderRenderState();
                var mc = alloc(Minecraft.class);
                var level = alloc(ClientLevel.class);
                put(level, ClientLevel.class, "worldBorder", border);
                put(mc, Minecraft.class, "level", level);
                var hud = alloc(Hud.class);
                put(hud, Hud.class, "minecraft", mc);
                var capture = alloc(Capture.class);
                var camera = alloc(net.minecraft.world.entity.decoration.ArmorStand.class);
                var frames = new JsonArray();
                for (var element : c.getAsJsonArray("ops")) {
                    var op = element.getAsJsonObject();
                    switch (op.get("op").getAsString()) {
                        case "tick" -> { for (int n = op.get("n").getAsInt(); n > 0; n--) border.tick(); }
                        case "lerp" -> border.lerpSizeBetween(op.get("from").getAsDouble(), op.get("to").getAsDouble(), op.get("ticks").getAsLong(), 0L);
                        case "size" -> border.setSize(op.get("size").getAsDouble());
                        case "center" -> border.setCenter(op.get("x").getAsDouble(), op.get("z").getAsDouble());
                        case "warning" -> { border.setWarningBlocks(op.get("blocks").getAsInt()); border.setWarningTime(op.get("time").getAsInt()); }
                        case "invalidate" -> renderer.invalidate();
                        case "frame" -> {
                            float partial = op.get("partial").getAsFloat();
                            var p = op.getAsJsonArray("camera");
                            var cam = new Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble());
                            double distance = op.get("renderDistance").getAsDouble();
                            millis = op.get("millis").getAsLong();
                            var frame = new JsonObject();
                            frame.add("bounds", doubles(border.getMinX(partial), border.getMinZ(partial), border.getMaxX(partial), border.getMaxZ(partial)));
                            frame.add("sizes", doubles(border.getSize(), border.getLerpTarget(), border.getLerpSpeed(), border.getDistanceToBorder(cam.x, cam.z)));
                            frame.addProperty("lerpTime", border.getLerpTime());
                            frame.addProperty("status", border.getStatus().name());
                            renderer.extract(border, partial, cam, distance, state);
                            frame.add("alpha", doubles(state.alpha));
                            frame.addProperty("tint", state.tint);
                            rebuilt = false;
                            renderer.prepare(state, cam, distance, op.get("depthFar").getAsFloat());
                            boolean drawn = state.alpha > 0.0;
                            frame.addProperty("drawn", drawn);
                            frame.addProperty("rebuilt", rebuilt);
                            if (drawn) {
                                frame.add("vertices", floats(lastVertices));
                                lastTransform = null;
                                prepareTransforms.invoke(renderer, state, cam);
                                var t = (DynamicGpuData.Transform) lastTransform;
                                var m = new org.joml.Matrix4f(t.textureMatrix());
                                frame.add("color", floats(t.colorModulator().x(), t.colorModulator().y(), t.colorModulator().z(), t.colorModulator().w()));
                                frame.add("offset", floats(t.modelOffset().x(), t.modelOffset().y(), t.modelOffset().z()));
                                frame.add("texture", floats(m.m30(), m.m31()));
                                sides.clear();
                                draw.invoke(renderer, state.closestBorder(cam.x, cam.z), distance, cam.x, cam.z, null, pass);
                                var s = new JsonArray();
                                for (int side : sides) s.add(side);
                                frame.add("sides", s);
                            }
                            put(camera, Entity.class, "position", cam);
                            put(hud, Hud.class, "vignetteBrightness", op.get("vignette").getAsFloat());
                            vignette.invoke(hud, capture, camera);
                            frame.addProperty("vignetteColor", vignetteColor);
                            frames.add(frame);
                        }
                        default -> throw new IllegalArgumentException(op.toString());
                    }
                }
                result.add("frames", frames);
            } catch (Throwable error) {
                var cause = error instanceof InvocationTargetException && error.getCause() != null ? error.getCause() : error;
                var sw = new java.io.StringWriter();
                cause.printStackTrace(new java.io.PrintWriter(sw));
                result.addProperty("error", sw.toString().lines().limit(10).reduce("", (x, y) -> x + y + " | "));
            }
            out.append(gson.toJson(result)).append('\n');
        }
        Files.writeString(Path.of(args[1]), out.toString(), StandardCharsets.UTF_8);
    }
}
