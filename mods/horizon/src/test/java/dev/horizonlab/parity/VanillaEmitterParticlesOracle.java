package dev.horizonlab.parity;

import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.SharedConstants;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.ParticleUtils;
import net.minecraft.util.RandomSource;
import net.minecraft.util.valueproviders.UniformInt;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;

/** Runs the real particle emitters of level events 3002 to 3005 (ParticleUtils
 * spawnParticlesOnBlockFaces and spawnParticlesAlongAxis, as LevelEventHandler
 * calls them) and LivingEntity.spawnItemParticles (eating, drinking, breaking)
 * against a stand-in level that records what they spawn. */
public final class VanillaEmitterParticlesOracle {
    static String d(double v) { return String.format("%016x", Double.doubleToRawLongBits(v)); }
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static final class Level0 extends ClientLevel {
        List<String> events;
        Level0() { super(null, null, null, null, 0, 0, null, false, 0L, 0); }
        @Override public void addParticle(ParticleOptions o, double x, double y, double z, double xd, double yd, double zd) {
            events.add("+ " + BuiltInRegistries.PARTICLE_TYPE.getKey(o.getType()).getPath() + " " + d(x) + " " + d(y) + " " + d(z)
                + " " + d(xd) + " " + d(yd) + " " + d(zd));
        }
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion(); Bootstrap.bootStrap(); ParityData.init();
        var u = Class.forName("sun.misc.Unsafe"); var unsafe = field(u, "theUnsafe").get(null);
        var allocate = u.getMethod("allocateInstance", Class.class);
        var level = (Level0) allocate.invoke(unsafe, Level0.class); level.events = new ArrayList<>();
        Field levelRandom = field(Level.class, "random");
        Field levelField = field(Entity.class, "level"), position = field(Entity.class, "position"), random = field(Entity.class, "random"),
            xRot = field(Entity.class, "xRot"), yRot = field(Entity.class, "yRot"), eyeHeight = field(Entity.class, "eyeHeight");
        try (var output = Files.newBufferedWriter(Path.of(args[1]))) {
            int n = 0;
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                output.write("== " + n++); output.newLine();
                level.events.clear();
                var r = RandomSource.create(c.get("seed").getAsLong());
                var p = c.getAsJsonArray("pos");
                var kind = c.get("kind").getAsString();
                if (kind.equals("item_use")) {
                    var entity = (Cow) allocate.invoke(unsafe, Cow.class);
                    levelField.set(entity, level);
                    position.set(entity, new Vec3(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble()));
                    random.set(entity, r);
                    xRot.setFloat(entity, c.get("pitch").getAsFloat());
                    yRot.setFloat(entity, c.get("yaw").getAsFloat());
                    eyeHeight.setFloat(entity, c.get("eye").getAsFloat());
                    var item = BuiltInRegistries.ITEM.getValue(Identifier.parse(c.get("item").getAsString()));
                    entity.spawnItemParticles(new ItemStack(item), c.get("count").getAsInt());
                } else {
                    levelRandom.set(level, r);
                    var pos = new BlockPos(p.get(0).getAsInt(), p.get(1).getAsInt(), p.get(2).getAsInt());
                    int axis = c.get("axis").getAsInt();
                    if (axis >= 0) ParticleUtils.spawnParticlesAlongAxis(Direction.Axis.VALUES[axis], level, pos, 0.125, ParticleTypes.ELECTRIC_SPARK, UniformInt.of(10, 19));
                    else ParticleUtils.spawnParticlesOnBlockFaces(level, pos, ParticleTypes.WAX_ON, UniformInt.of(3, 5));
                }
                for (var e : level.events) { output.write(e); output.newLine(); }
                output.write("next " + r.nextLong()); output.newLine();
            }
        }
    }
}
