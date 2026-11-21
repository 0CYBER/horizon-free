package dev.horizonlab.parity;

import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.UUID;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.util.ProblemReporter;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.Interaction;
import net.minecraft.world.level.storage.TagValueOutput;
import net.minecraft.world.level.storage.ValueOutput;
import net.minecraft.world.phys.Vec3;

/**
 * Runs the shipped `Interaction.interact` on a server, once per `response` value, and
 * prints what the click answered and what the entity then saves. Only the level's game
 * time and the clicker's id are fixture boundaries: a client's own prediction
 * (`getResponse`), `skipAttackInteraction` and the metadata defaults around them are
 * outside this oracle.
 */
public final class VanillaInteractionClickOracle {
    /** The player a server's interact packet names, as `UUIDUtil` saves it: four ints. */
    static final UUID PLAYER = new UUID(0x0123456789abcdefL, 0x0123456789abcdefL);
    static Object unsafe;
    static Method allocate;

    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }

    static Method declared(Class<?> type, String name, Class<?>... parameters) throws Exception {
        var method = type.getDeclaredMethod(name, parameters);
        method.setAccessible(true);
        return method;
    }

    /** `ServerLevel` with the clock pinned: the only level answers the entity and its click ask for. */
    static class World extends ServerLevel {
        long gameTime;

        World() {
            super(null, null, null, null, null, null, false, 0L, null, false);
        }

        @Override
        public long getGameTime() {
            return this.gameTime;
        }

        @Override
        public boolean isClientSide() {
            return false;
        }

        @Override
        public int getNextEntityId() {
            return 1;
        }
    }

    /** The clicking player: a server player's id, and nothing else. */
    static class Actor extends ServerPlayer {
        Actor() {
            super(null, null, null, null);
        }

        @Override
        public UUID getUUID() {
            return PLAYER;
        }
    }

    static String bits(float value) {
        return String.format(Locale.ROOT, "%08x", Float.floatToRawIntBits(value));
    }

    static String ints(int[] values) {
        var text = new StringBuilder();
        for (int value : values) {
            text.append(text.isEmpty() ? "" : ",").append(value);
        }
        return "[" + text + "]";
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var runtime = Class.forName("sun.misc.Unsafe");
        var instance = runtime.getDeclaredField("theUnsafe");
        instance.setAccessible(true);
        unsafe = instance.get(null);
        allocate = runtime.getMethod("allocateInstance", Class.class);
        var setWidth = declared(Interaction.class, "setWidth", float.class);
        var setHeight = declared(Interaction.class, "setHeight", float.class);
        var setResponse = declared(Interaction.class, "setResponse", boolean.class);
        var save = declared(Interaction.class, "addAdditionalSaveData", ValueOutput.class);
        var world = make(World.class);
        var actor = make(Actor.class);
        var out = new StringBuilder("[");
        // One case per `response`: on a server the flag reaches no answer, only the record.
        for (boolean response : new boolean[]{false, true}) {
            world.gameTime = response ? 8642L : 4321L;
            var entity = new Interaction(EntityTypes.INTERACTION, world);
            setWidth.invoke(entity, response ? 2.5F : 1.5F);
            setHeight.invoke(entity, response ? 3.25F : 2.25F);
            setResponse.invoke(entity, response);
            var answer = entity.interact(actor, InteractionHand.MAIN_HAND, Vec3.ZERO);
            var saved = TagValueOutput.createWithContext(ProblemReporter.DISCARDING, ParityData.access);
            save.invoke(entity, saved);
            CompoundTag tag = saved.buildResult();
            var record = tag.getCompound("interaction").orElse(new CompoundTag());
            if (out.length() > 1) {
                out.append(',');
            }
            out.append("{\"case\":\"response_").append(response).append('"')
                .append(",\"result\":\"").append(answer == InteractionResult.CONSUME ? "consume" : String.valueOf(answer)).append('"')
                .append(",\"width\":\"").append(bits(tag.getFloat("width").orElse(Float.NaN))).append('"')
                .append(",\"height\":\"").append(bits(tag.getFloat("height").orElse(Float.NaN))).append('"')
                .append(",\"response\":").append(tag.getByteOr("response", (byte) -1))
                .append(",\"interaction\":{\"player\":").append(ints(record.getIntArray("player").orElse(new int[0])))
                .append(",\"timestamp\":").append(record.getLong("timestamp").orElse(-1L)).append("}}");
        }
        out.append(']');
        Files.writeString(Path.of(args[0]), out + "\n");
        System.exit(0);
    }
}