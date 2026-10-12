package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.resources.sounds.*;
import net.minecraft.client.sounds.ChannelAccess;
import net.minecraft.client.sounds.SoundEngine;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundEvent;
import net.minecraft.sounds.SoundSource;
import net.minecraft.tags.FluidTags;
import net.minecraft.tags.TagKey;
import net.minecraft.util.RandomSource;
import net.minecraft.world.attribute.*;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.BubbleColumnBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.phys.Vec3;

/**
 * Runs the local player's real ambient sound path over the real SoundEngine
 * (VanillaSoundRig): LocalPlayer.updateIsUnderwater (enter/exit sounds through
 * ClientLevel.playLocalSound and the UnderLiquidAmbientSoundInstance), the
 * UnderwaterAmbientSoundHandler and the BiomeAmbientSoundsHandler, then
 * SoundManager.tick, once per client tick. Inputs: the AMBIENT_SOUNDS value
 * (a constant attribute layer), the player's position, eye height, whether
 * its eyes are in water, spectator and removed flags, and the level's sky and
 * block brightness (a hash of the position). Prints the level random's next
 * long (without drawing it), the handlers' state and every playing instance.
 */
public final class VanillaAmbientOracle {
    public static final class Actor extends LocalPlayer {
        boolean eyeInWater, spectator;

        Actor() {
            super(null, null, null, null, null, null, false, null, null);
        }

        @Override
        public boolean isEyeInFluid(final TagKey<Fluid> type) {
            return type == FluidTags.WATER && eyeInWater;
        }

        @Override
        public boolean isSpectator() {
            return spectator;
        }
    }

    public static final class Lvl extends ClientLevel {
        int mode;
        final Map<BlockPos, Boolean> columns = new HashMap<>();

        Lvl() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override
        public int getBrightness(final LightLayer layer, final BlockPos pos) {
            return light(mode, pos.getX(), pos.getY(), pos.getZ())[layer == LightLayer.SKY ? 0 : 1];
        }

        @Override
        public BlockState getBlockState(final BlockPos pos) {
            Boolean drag = columns.get(pos);
            return drag == null ? Blocks.AIR.defaultBlockState() : Blocks.BUBBLE_COLUMN.defaultBlockState().setValue(BubbleColumnBlock.DRAG_DOWN, drag);
        }

        @Override
        public boolean hasChunksAt(final int x0, final int y0, final int z0, final int x1, final int y1, final int z1) {
            return true;
        }
    }

    /** The scripted brightness: (sky, block) for a light mode and block. */
    static int[] light(int mode, int x, int y, int z) {
        int h = x * 73856093 ^ y * 19349663 ^ z * 83492791;
        return switch (mode) {
            case 0 -> new int[]{15, 0};
            case 1 -> new int[]{0, 0};
            case 2 -> new int[]{0, (h >>> 4) & 15};
            default -> new int[]{(h & 3) == 0 ? (h >>> 8) & 15 : 0, (h >>> 12) & 15};
        };
    }

    /** The player's box (EntityDimensions.makeBoundingBox, width 0.6F). */
    static AABB box(Vec3 at, float height) {
        return new AABB(at.x - 0.3F, at.y, at.z - 0.3F, at.x + 0.3F, at.y + height, at.z + 0.3F);
    }

    static float f(String hex) {
        return Float.intBitsToFloat(Integer.parseUnsignedInt(hex, 16));
    }

    static double d(String hex) {
        return Double.longBitsToDouble(Long.parseUnsignedLong(hex, 16));
    }

    static Holder<SoundEvent> event(String id) {
        return Holder.direct(SoundEvent.createVariableRangeEvent(Identifier.parse(id)));
    }

    /** "loop|- mood|- addition..." with mood "sound,delay,extent,offsetbits" and additions "sound,chancebits". */
    static AmbientSounds sounds(String[] a) {
        Optional<Holder<SoundEvent>> loop = a[1].equals("-") ? Optional.empty() : Optional.of(event(a[1]));
        Optional<AmbientMoodSettings> mood = Optional.empty();
        if (!a[2].equals("-")) {
            String[] m = a[2].split(",");
            mood = Optional.of(new AmbientMoodSettings(event(m[0]), Integer.parseInt(m[1]), Integer.parseInt(m[2]), d(m[3])));
        }
        var additions = new ArrayList<AmbientAdditionsSettings>();
        for (int i = 3; i < a.length; i++) {
            String[] p = a[i].split(",");
            additions.add(new AmbientAdditionsSettings(event(p[0]), d(p[1])));
        }
        return new AmbientSounds(loop, mood, additions);
    }

    /** LegacyRandomSource's next nextLong(), read from its state. */
    static long peek(RandomSource random) throws Exception {
        long s = ((java.util.concurrent.atomic.AtomicLong) VanillaSoundRig.get(random,
            Class.forName("net.minecraft.world.level.levelgen.LegacyRandomSource"), "seed")).get();
        long m = 0x5DEECE66DL, mask = (1L << 48) - 1;
        long s1 = (s * m + 0xBL) & mask;
        long s2 = (s1 * m + 0xBL) & mask;
        return ((long) (int) (s1 >>> 16) << 32) + (int) (s2 >>> 16);
    }

    static String bits(double v) {
        return String.format("%016x", Double.doubleToRawLongBits(v));
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var rig = new VanillaSoundRig(Path.of(args[0]));
        var mc = VanillaSoundRig.make(Minecraft.class);
        VanillaSoundRig.set(mc, Minecraft.class, "soundManager", rig.manager);
        var renderer = VanillaSoundRig.make(GameRenderer.class);
        VanillaSoundRig.set(renderer, GameRenderer.class, "mainCamera", VanillaSoundRig.make(Camera.class));
        VanillaSoundRig.set(VanillaSoundRig.get(renderer, GameRenderer.class, "mainCamera"), Camera.class, "position", Vec3.ZERO);
        VanillaSoundRig.set(mc, Minecraft.class, "gameRenderer", renderer);
        Method update = LocalPlayer.class.getDeclaredMethod("updateIsUnderwater");
        update.setAccessible(true);
        var instanceToChannel = VanillaSoundRig.field(SoundEngine.class, "instanceToChannel");
        Lvl level = null;
        Actor actor = null;
        UnderwaterAmbientSoundHandler underwater = null;
        BiomeAmbientSoundsHandler biome = null;
        BubbleColumnAmbientSoundHandler bubble = null;
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2])))) {
            for (String line : Files.readAllLines(Path.of(args[1]))) {
                out.println(line);
                String[] a = line.split(" ");
                switch (a[0]) {
                    case "start" -> {
                        // A new level and player: fresh handlers on a seeded level random.
                        rig.manager.stop();
                        level = VanillaSoundRig.make(Lvl.class);
                        VanillaSoundRig.set(level, Level.class, "random", RandomSource.create(Long.parseLong(a[1])));
                        VanillaSoundRig.set(level, ClientLevel.class, "minecraft", mc);
                        VanillaSoundRig.set(level, ClientLevel.class, "environmentAttributes", EnvironmentAttributeSystem.builder().build());
                        actor = VanillaSoundRig.make(Actor.class);
                        VanillaSoundRig.set(actor, Entity.class, "level", level);
                        VanillaSoundRig.set(actor, Entity.class, "position", Vec3.ZERO);
                        VanillaSoundRig.set(actor, Entity.class, "bb", box(Vec3.ZERO, 1.8F));
                        VanillaSoundRig.set(level, Lvl.class, "columns", new HashMap<BlockPos, Boolean>());
                        VanillaSoundRig.set(actor, LocalPlayer.class, "minecraft", mc);
                        underwater = new UnderwaterAmbientSoundHandler(actor, rig.manager);
                        bubble = new BubbleColumnAmbientSoundHandler(actor);
                        biome = new BiomeAmbientSoundsHandler(actor, rig.manager);
                    }
                    case "sounds" -> {
                        AmbientSounds value = sounds(a);
                        var system = EnvironmentAttributeSystem.builder()
                            .addConstantLayer(EnvironmentAttributes.AMBIENT_SOUNDS, base -> value)
                            .build();
                        VanillaSoundRig.set(level, ClientLevel.class, "environmentAttributes", system);
                    }
                    case "player" -> {
                        // player x y z eyeHeight eyeInWater spectator removed lightMode
                        VanillaSoundRig.set(actor, Entity.class, "position", new Vec3(d(a[1]), d(a[2]), d(a[3])));
                        VanillaSoundRig.set(actor, Entity.class, "eyeHeight", f(a[4]));
                        actor.eyeInWater = a[5].equals("1");
                        actor.spectator = a[6].equals("1");
                        VanillaSoundRig.set(actor, Entity.class, "removalReason", a[7].equals("1") ? Entity.RemovalReason.DISCARDED : null);
                        level.mode = Integer.parseInt(a[8]);
                        var at = new Vec3(d(a[1]), d(a[2]), d(a[3]));
                        VanillaSoundRig.set(actor, Entity.class, "bb", box(at, f(a[9])));
                        level.columns.clear();
                        for (int i = 10; i < a.length; i++) {
                            String[] c = a[i].split(",");
                            level.columns.put(new BlockPos(Integer.parseInt(c[0]), Integer.parseInt(c[1]), Integer.parseInt(c[2])), c[3].equals("1"));
                        }
                    }
                    case "volume" -> {
                        rig.volume(SoundSource.values()[Integer.parseInt(a[1])], f(a[2]));
                        rig.manager.refreshCategoryVolume(SoundSource.values()[Integer.parseInt(a[1])]);
                    }
                    case "tick" -> {
                        update.invoke(actor);
                        underwater.tick();
                        bubble.tick();
                        biome.tick();
                        rig.manager.tick(false);
                    }
                    default -> throw new IllegalArgumentException(line);
                }
                out.printf("  rand=%d mood=%08x underwater=%s delay=%d%n", peek(level.getRandom()), Float.floatToRawIntBits(biome.getMoodiness()),
                    actor.isUnderWater(), (int) VanillaSoundRig.get(underwater, UnderwaterAmbientSoundHandler.class, "tickDelay"));
                var loops = (Map<Holder<SoundEvent>, BiomeAmbientSoundsHandler.LoopSoundInstance>) VanillaSoundRig.get(biome,
                    BiomeAmbientSoundsHandler.class, "loopSounds");
                for (var e : loops.entrySet()) {
                    var loop = e.getValue();
                    out.printf("  loop %s fade=%d direction=%d stopped=%s active=%s%n", e.getKey().value().location(),
                        (int) VanillaSoundRig.get(loop, BiomeAmbientSoundsHandler.LoopSoundInstance.class, "fade"),
                        (int) VanillaSoundRig.get(loop, BiomeAmbientSoundsHandler.LoopSoundInstance.class, "fadeDirection"), loop.isStopped(),
                        rig.manager.isActive(loop));
                }
                var playing = new ArrayList<String>();
                for (SoundInstance i : ((Map<SoundInstance, ChannelAccess.ChannelHandle>) instanceToChannel.get(rig.engine)).keySet()) {
                    float volume = (float) VanillaSoundRig.get(i, AbstractSoundInstance.class, "volume");
                    playing.add(String.format("  playing %s volume=%08x looping=%s relative=%s at=%s,%s,%s stopped=%s", i.getIdentifier(),
                        Float.floatToRawIntBits(volume), i.isLooping(), i.isRelative(), bits(i.getX()), bits(i.getY()), bits(i.getZ()),
                        i instanceof TickableSoundInstance t && t.isStopped()));
                }
                Collections.sort(playing);
                for (String p : playing) out.println(p);
                rig.flush(out);
            }
        }
        System.exit(0);
    }
}
