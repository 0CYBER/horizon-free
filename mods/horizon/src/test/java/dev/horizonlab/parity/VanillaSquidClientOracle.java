package dev.horizonlab.parity;

import com.google.gson.JsonParser;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Proxy;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.Holder;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.animal.squid.Squid;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

/** Runs the real Squid.aiStep, with unrelated movement/world services controlled. */
public final class VanillaSquidClientOracle {
    static Field field(Class<?> owner, String name) throws Exception {
        var f = owner.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static final class Shell extends Squid {
        Level world; boolean water;
        Shell() { super(EntityTypes.SQUID, null); }
        @Override public Level level() { return world; }
        @Override public boolean isAgeLocked() { return true; }
        @Override public boolean isInWater() { return water; }
        @Override protected boolean isImmobile() { return true; }
        @Override protected void applyInput() {}
        @Override public boolean isEffectiveAi() { return false; }
        @Override public boolean isFallFlying() { return false; }
        @Override public boolean hasEffect(Holder<MobEffect> effect) { return false; }
        @Override public LivingEntity getControllingPassenger() { return null; }
        @Override public void calculateEntityAnimation(boolean flying) {}
        @Override protected void pushEntities() {}
    }
    static String bits(float value) { return String.format("%08x", Float.floatToIntBits(value)); }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        var uf = field(u, "theUnsafe"); var unsafe = uf.get(null);
        var allocate = u.getMethod("allocateInstance", Class.class);
        var world = (ClientLevel) allocate.invoke(unsafe, ClientLevel.class);
        field(Level.class, "isClientSide").setBoolean(world, true);
        String[] names = {"tentacleMovement", "tentacleAngle", "xBodyRot", "zBodyRot", "tentacleSpeed", "rotateSpeed"};
        Field[] values = new Field[names.length];
        for (int i = 0; i < names.length; i++) values[i] = field(Squid.class, names[i]);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                var m = (Shell) allocate.invoke(unsafe, Shell.class);
                m.world = world; field(Entity.class, "level").set(m, world); m.water = row.get("water").getAsBoolean();
                field(Entity.class, "type").set(m, EntityTypes.SQUID);
                field(LivingEntity.class, "equipment").set(m, new EntityEquipment());
                field(Entity.class, "interpolationHandler").set(m, Proxy.newProxyInstance(
                    InterpolationHandler.class.getClassLoader(), new Class<?>[]{InterpolationHandler.class},
                    (proxy, method, arguments) -> method.getName().equals("hasActiveInterpolation") ? true : null));
                m.setBoundingBox(new AABB(0, 0, 0, 1, 1, 1));
                m.setDeltaMovement(new Vec3(row.get("x").getAsDouble(), row.get("y").getAsDouble(), row.get("z").getAsDouble()));
                m.yBodyRot = row.get("body").getAsFloat();
                for (int i = 0; i < names.length; i++) values[i].setFloat(m, row.getAsJsonArray("v").get(i).getAsFloat());
                if (row.get("reset").getAsBoolean()) m.handleEntityEvent((byte)19);
                m.aiStep();
                out.print(bits(m.yBodyRot));
                for (var f : values) out.print(" " + bits(f.getFloat(m)));
                out.println();
            }
        }
    }
}
