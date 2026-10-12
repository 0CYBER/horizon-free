package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Options;
import net.minecraft.client.resources.sounds.*;
import net.minecraft.client.sounds.*;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.contents.TranslatableContents;
import net.minecraft.resources.Identifier;
import net.minecraft.server.packs.*;
import net.minecraft.server.packs.repository.PackSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.sounds.SoundSource;
import net.minecraft.util.RandomSource;
import net.minecraft.util.profiling.InactiveProfiler;

/**
 * Builds vanilla's sound event registry with the real SoundManager.prepare
 * (sounds.json read through a resource manager over the client asset root,
 * files validated against the listed .ogg resources) and Preparations.apply,
 * then resolves SimpleSoundInstances: getOrResolve with a seeded random,
 * getVolume/getPitch, and the instance random's next draw (to fix the number
 * of draws). Lines: "event seed volume pitch".
 */
public final class VanillaSoundEventOracle {
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    static String bits(float v) {
        return String.format("%08x", Float.floatToRawIntBits(v));
    }

    static String sentinel(Sound sound) {
        if (sound == SoundManager.EMPTY_SOUND) return "empty";
        if (sound == SoundManager.INTENTIONALLY_EMPTY_SOUND) return "intentionally_empty";
        return "-";
    }

    /** The real registry, prepared from an asset root. */
    @SuppressWarnings("unchecked")
    static SoundManager load(Path root) throws Exception {
        var info = new PackLocationInfo("horizon", Component.literal("horizon"), PackSource.BUILT_IN, Optional.empty());
        ResourceManager resources = new MultiPackResourceManager(PackType.CLIENT_RESOURCES, List.of(new PathPackResources(info, root)));
        var u = Class.forName("sun.misc.Unsafe");
        Object unsafe = field(u, "theUnsafe").get(null);
        Options options = (Options) u.getMethod("allocateInstance", Class.class).invoke(unsafe, Options.class);
        var manager = new SoundManager(options);
        Method prepare = SoundManager.class.getDeclaredMethod("prepare", ResourceManager.class, net.minecraft.util.profiling.ProfilerFiller.class);
        prepare.setAccessible(true);
        Object preparations = prepare.invoke(manager, resources, InactiveProfiler.INSTANCE);
        Method apply = preparations.getClass().getDeclaredMethod("apply", Map.class, Map.class, SoundEngine.class);
        apply.setAccessible(true);
        apply.invoke(preparations, field(SoundManager.class, "registry").get(manager), field(SoundManager.class, "soundCache").get(manager),
            field(SoundManager.class, "soundEngine").get(manager));
        return manager;
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var manager = load(Path.of(args[0]));
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2])))) {
            var names = new ArrayList<String>();
            for (var id : manager.getAvailableSounds()) names.add(id.toString());
            Collections.sort(names);
            for (String name : names) out.println("registered " + name);
            for (String line : Files.readAllLines(Path.of(args[1]))) {
                String[] f = line.split(" ");
                long seed = Long.parseLong(f[1]);
                float volume = Float.intBitsToFloat(Integer.parseUnsignedInt(f[2], 16));
                float pitch = Float.intBitsToFloat(Integer.parseUnsignedInt(f[3], 16));
                RandomSource random = RandomSource.create(seed);
                var instance = new SimpleSoundInstance(Identifier.parse(f[0]), SoundSource.MASTER, volume, pitch, random, false, 0,
                    SoundInstance.Attenuation.LINEAR, 0.0, 0.0, 0.0, false);
                WeighedSoundEvents event = instance.getOrResolve(manager);
                Sound sound = instance.getSound();
                StringBuilder sb = new StringBuilder(line);
                if (event == null) {
                    sb.append(" event=none");
                } else {
                    String subtitle = "-";
                    if (event.getSubtitle() != null && event.getSubtitle().getContents() instanceof TranslatableContents t) subtitle = t.getKey();
                    sb.append(" event weight=").append(event.getWeight()).append(" subtitle=").append(subtitle);
                }
                float v = instance.getVolume();
                float p = instance.getPitch();
                sb.append(" sound=").append(sound.getLocation()).append(" path=").append(sound.getPath())
                    .append(" sentinel=").append(sentinel(sound))
                    .append(" weight=").append(sound.getWeight())
                    .append(" type=").append(sound.getType().name())
                    .append(" stream=").append(sound.shouldStream())
                    .append(" preload=").append(sound.shouldPreload())
                    .append(" attenuation=").append(sound.getAttenuationDistance())
                    .append(" volume=").append(bits(v))
                    .append(" pitch=").append(bits(p))
                    .append(" range=").append(bits(sound.getAttenuationDistance(v)))
                    .append(" next=").append(random.nextLong());
                out.println(sb);
            }
        }
        System.exit(0);
    }
}
