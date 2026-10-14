package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.RandomSource;
import net.minecraft.world.item.FireChargeItem;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.Level;

/**
 * Original oracle for the fire charge's use sound: the real private
 * FireChargeItem.playSound (used for both a lit campfire or candle and a
 * placed fire) on a level stand-in whose random is seeded from the case,
 * recording the sound, its volume and pitch and the level random's next long
 * (which shows how many values the sound drew).
 */
public final class VanillaFireStarterSoundOracle {
    static final StringBuilder sounds = new StringBuilder();

    static final class World extends ServerLevel {
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        @Override public void playSeededSound(net.minecraft.world.entity.Entity p, double x, double y, double z,
                net.minecraft.core.Holder<net.minecraft.sounds.SoundEvent> s, net.minecraft.sounds.SoundSource c, float v, float pitch, long seed) {
            sounds.append(s.unwrapKey().orElseThrow().identifier()).append(' ')
                .append(Float.floatToIntBits(v)).append(' ').append(Float.floatToIntBits(pitch));
        }
    }

    static void put(Object unsafe, Object target, String name, Object value) throws Exception {
        Field f = Level.class.getDeclaredField(name);
        long offset = (long) unsafe.getClass().getMethod("objectFieldOffset", Field.class).invoke(unsafe, f);
        unsafe.getClass().getMethod("putObject", Object.class, long.class, Object.class).invoke(unsafe, target, offset, value);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        Field theUnsafe = u.getDeclaredField("theUnsafe");
        theUnsafe.setAccessible(true);
        Object unsafe = theUnsafe.get(null);
        World world = (World) u.getMethod("allocateInstance", Class.class).invoke(unsafe, World.class);
        Method play = FireChargeItem.class.getDeclaredMethod("playSound", Level.class, BlockPos.class);
        play.setAccessible(true);
        var item = (FireChargeItem) Items.FIRE_CHARGE;
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                long seed = Long.parseLong(line.trim());
                RandomSource random = RandomSource.create(seed);
                put(unsafe, world, "random", random);
                put(unsafe, world, "soundSeedGenerator", RandomSource.create(0));
                sounds.setLength(0);
                play.invoke(item, world, new BlockPos(3, 64, -7));
                out.println(sounds + " " + random.nextLong());
            }
        }
        System.exit(0);
    }
}
