package dev.horizonlab.parity;

import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.core.Holder;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.particles.*;
import net.minecraft.world.effect.*;

/** Exports and checks the pinned registry's inherited, pure particle factories. */
public final class VanillaEffectParticlesOracle {
    static Field field(Class<?> c, String name) throws Exception {
        var f = c.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static JsonObject particle(ParticleOptions p) throws Exception {
        var result = new JsonObject();
        result.addProperty("type", BuiltInRegistries.PARTICLE_TYPE.getKey(p.getType()).toString());
        if (p.getClass() == ColorParticleOption.class) {
            result.addProperty("color", Integer.toUnsignedLong(field(ColorParticleOption.class, "color").getInt(p)));
        } else if (!(p instanceof SimpleParticleType)) throw new AssertionError("unsupported registered particle: " + p.getClass());
        return result;
    }
    static JsonObject export() throws Exception {
        var root = new JsonObject();
        root.addProperty("data_version", net.minecraft.SharedConstants.getCurrentVersion().dataVersion().version());
        int alpha = field(MobEffect.class, "AMBIENT_ALPHA").getInt(null);
        if (alpha != 38) throw new AssertionError("ambient alpha changed");
        root.addProperty("ambient_alpha", alpha);
        var definitions = new JsonArray();
        for (var effect : BuiltInRegistries.MOB_EFFECT) {
            var holder = BuiltInRegistries.MOB_EFFECT.wrapAsHolder(effect);
            if (effect.getClass().getMethod("createParticleOptions", MobEffectInstance.class).getDeclaringClass() != MobEffect.class)
                throw new AssertionError("registered factory override requires inspection: " + effect.getClass());
            // MobEffect's two constructors assign their sole captured argument
            // to a final factory. The pinned bytecode uses only that argument
            // and the ordinary instance's ambient flag; it has no world access.
            Object factory = field(MobEffect.class, "particleFactory").get(effect);
            Field[] captures = factory.getClass().getDeclaredFields();
            if (captures.length != 1) throw new AssertionError("factory capture layout changed");
            captures[0].setAccessible(true);
            Object captured = captures[0].get(factory);
            var def = new JsonObject(); def.addProperty("id", holder.getRegisteredName());
            if (captured instanceof Integer color) {
                def.addProperty("factory", "color"); def.addProperty("color", Integer.toUnsignedLong(color));
            } else if (captured instanceof ParticleOptions p) {
                if (!(p instanceof SimpleParticleType)) throw new AssertionError("constant option identity needs a new export schema");
                def.addProperty("factory", "constant"); def.add("particle", particle(p));
            } else throw new AssertionError("unexpected factory capture: " + captured);
            for (boolean ambient : new boolean[] {false, true}) {
                for (int amplifier : new int[] {0, 1, 127, 255}) {
                    var instance = new MobEffectInstance(holder, 200, amplifier, ambient, true, true);
                    var a = effect.createParticleOptions(instance); var b = effect.createParticleOptions(instance);
                    if (captured instanceof Integer color) {
                        int expected = ((ambient ? alpha : 255) << 24) | (color & 0xffffff);
                        if (a.getClass() != ColorParticleOption.class || a.getType() != ParticleTypes.ENTITY_EFFECT
                            || field(ColorParticleOption.class, "color").getInt(a) != expected || a == b || a.equals(b))
                            throw new AssertionError("color factory value/identity changed");
                    } else if (a != captured || b != captured) throw new AssertionError("constant identity changed");
                }
            }
            definitions.add(def);
        }
        root.add("effects", definitions); return root;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion(); net.minecraft.server.Bootstrap.bootStrap();
        Files.writeString(Path.of(args[0]), export().toString());
    }
}
