package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.NonNullList;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.Container;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.LegacyRandomSource;

/**
 * Drives vanilla `DispenserBlock.dispenseFrom` (and the dropper's) over the
 * headless level of {@link VanillaBlockUpdateOracle}, with entities: the
 * dispenser holds the case's items, target entities stand in front, and the
 * result lists block changes, container contents, spawned entities with their
 * position and motion, the targets' state afterwards, level events, sounds
 * (those the client plays for level events included), game events and the
 * level random. Every case runs twice: numbers that differ between the runs
 * come from an entity's own unseeded random and are marked with "~".
 * Used by tools/test_dispenser_parity.py.
 */
public final class VanillaDispenserOracle {
    /** The sound LevelEventHandler plays for a level event a dispenser can cause, or null. */
    static String eventSound(int id, int data) {
        return switch (id) {
            case 1000 -> "minecraft:block.dispenser.dispense";
            case 1001 -> "minecraft:block.dispenser.fail";
            case 1002 -> "minecraft:block.dispenser.launch";
            case 1004 -> "minecraft:entity.firework_rocket.shoot";
            case 1018 -> "minecraft:entity.blaze.shoot";
            case 1051 -> "minecraft:entity.wind_charge.throw";
            case 1505 -> "minecraft:item.bone_meal.use";
            case 1009 -> data == 0 ? "minecraft:block.fire.extinguish" : data == 1 ? "minecraft:block.generic.extinguish_fire" : null;
            case 2001 -> {
                BlockState state = Block.stateById(data);
                yield state.isAir() ? null : state.getSoundType().getBreakSound().location().toString();
            }
            default -> null;
        };
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        VanillaBlockUpdateOracle.AIR = Blocks.AIR.defaultBlockState();
        Path resources = Path.of(args[0]);
        VanillaLevelShapeOracle.bindFluidTags(resources.resolve("data/minecraft/tags/fluid"));
        VanillaBlockUpdateOracle.OVERWORLD_TYPE = VanillaBlockUpdateOracle.loadOverworld();
        var allocator = Class.forName("sun.misc.Unsafe");
        var field = allocator.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        VanillaBlockUpdateOracle.unsafe = field.get(null);
        VanillaBlockUpdateOracle.allocate = allocator.getMethod("allocateInstance", Class.class);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[2]))); var lines = Files.lines(Path.of(args[1]))) {
            int index = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                List<String> a = attempt(f), b = attempt(f);
                out.print("case " + index++ + "\n");
                for (String s : merge(a, b)) out.print(s + "\n");
            }
        }
    }

    static List<String> attempt(JsonObject f) {
        List<String> result = new ArrayList<>();
        try {
            run(f, result);
        } catch (Throwable e) {
            Throwable c = e;
            while (c.getCause() != null) c = c.getCause();
            if (System.getenv("ORACLE_TRACE") != null) c.printStackTrace();
            StackTraceElement[] st = c.getStackTrace();
            result.clear();
            result.add("error " + c.getClass().getSimpleName() + " " + (st.length > 0 ? st[0].toString() : "") + " " + c.getMessage());
        }
        return result;
    }

    /** Lines equal in both runs stay; numbers that differ are marked "~" (from an unseeded entity random). */
    static List<String> merge(List<String> a, List<String> b) {
        if (a.size() != b.size()) return a;
        List<String> out = new ArrayList<>();
        for (int i = 0; i < a.size(); i++) {
            String x = a.get(i), y = b.get(i);
            if (x.equals(y)) {
                out.add(x);
                continue;
            }
            String[] p = x.split(" "), q = y.split(" ");
            if (p.length != q.length) {
                out.add(x);
                continue;
            }
            for (int j = 0; j < p.length; j++) if (!p[j].equals(q[j])) p[j] = "~" + p[j];
            out.add(String.join(" ", p));
        }
        return out;
    }

    static String num(double v) {
        return String.format(Locale.ROOT, "%.6f", v);
    }

    static ItemStack stack(String snbt) throws Exception {
        return ParityData.stack(snbt);
    }

    /** The item list of a container block entity (dispensers, droppers, chests, hoppers, barrels...). */
    @SuppressWarnings("unchecked")
    static NonNullList<ItemStack> slots(BlockEntity be) throws Exception {
        for (Class<?> c = be.getClass(); c != null; c = c.getSuperclass()) {
            for (Field fd : c.getDeclaredFields()) {
                if (fd.getType() == NonNullList.class && !java.lang.reflect.Modifier.isStatic(fd.getModifiers())) {
                    fd.setAccessible(true);
                    return (NonNullList<ItemStack>) fd.get(be);
                }
            }
        }
        throw new IllegalStateException("no slots in " + be);
    }

    /**
     * The chunk source bone meal reaches for: saplings, grass and moss place
     * their features with `getChunkSource().getGenerator()`, which none of
     * them reads beyond passing it along.
     */
    static final class Chunks extends net.minecraft.server.level.ServerChunkCache {
        static net.minecraft.world.level.chunk.ChunkGenerator generator;

        private Chunks() {
            super(null, null, null, null, null, null, 0, 0, false, null);
        }

        @Override
        public net.minecraft.world.level.chunk.ChunkGenerator getGenerator() {
            return generator;
        }

        /** Leads send entity link packets; nobody is tracking. */
        @Override
        public void sendToTrackingPlayers(final Entity entity, final net.minecraft.network.protocol.Packet<? super net.minecraft.network.protocol.game.ClientGamePacketListener> packet) {
        }
    }

    static Method dispenseFrom(Class<?> type) throws Exception {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                Method m = c.getDeclaredMethod("dispenseFrom", ServerLevel.class, BlockState.class, BlockPos.class);
                m.setAccessible(true);
                return m;
            } catch (NoSuchMethodException ignored) {
            }
        }
        throw new IllegalStateException("no dispenseFrom on " + type);
    }

    static Entity target(VanillaBlockUpdateOracle.World w, JsonObject e) throws Exception {
        EntityType<?> type = BuiltInRegistries.ENTITY_TYPE.getValue(Identifier.parse(e.get("type").getAsString()));
        Entity ent = type.create(w, EntitySpawnReason.COMMAND);
        var p = e.getAsJsonArray("pos");
        ent.snapTo(p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble(), e.has("yaw") ? e.get("yaw").getAsFloat() : 0f, 0f);
        if (e.has("baby") && ent instanceof AgeableMob ageable) ageable.setAge(-24000);
        if (e.has("tame")) {
            if (ent instanceof net.minecraft.world.entity.animal.equine.AbstractHorse horse) horse.setTamed(true);
            if (ent instanceof TamableAnimal tamable) tamable.setTame(true, false);
        }
        if (e.has("sheared")) {
            if (ent instanceof net.minecraft.world.entity.animal.sheep.Sheep sheep) sheep.setSheared(true);
            if (ent instanceof net.minecraft.world.entity.monster.skeleton.Bogged bogged) bogged.setSheared(true);
            if (ent instanceof net.minecraft.world.entity.animal.golem.SnowGolem golem) golem.setPumpkin(false);
        }
        if (e.has("chest") && ent instanceof net.minecraft.world.entity.animal.equine.AbstractChestedHorse chested) chested.setChest(true);
        if (e.has("loot") && ent instanceof Mob mob) mob.setCanPickUpLoot(true);
        if (e.has("brown") && ent instanceof net.minecraft.world.entity.animal.cow.MushroomCow cow) {
            Method variant = net.minecraft.world.entity.animal.cow.MushroomCow.class.getDeclaredMethod("setVariant", net.minecraft.world.entity.animal.cow.MushroomCow.Variant.class);
            variant.setAccessible(true);
            variant.invoke(cow, net.minecraft.world.entity.animal.cow.MushroomCow.Variant.BROWN);
        }
        // A target has lived in the level for a while: past its first tick, so
        // equipping it plays the equip sound.
        VanillaBlockUpdateOracle.set(Entity.class, ent, "firstTick", false);
        if (e.has("equip") && ent instanceof LivingEntity living) {
            for (var slot : e.getAsJsonObject("equip").entrySet()) {
                living.setItemSlot(EquipmentSlot.byName(slot.getKey()), stack(slot.getValue().getAsString()));
            }
            // A target that has ticked since it was equipped has run
            // collectEquipmentChanges (a sulfur cube takes its archetype then).
            Map<EquipmentSlot, ItemStack> last = new EnumMap<>(EquipmentSlot.class);
            for (EquipmentSlot slot : EquipmentSlot.values()) last.put(slot, ItemStack.EMPTY);
            Method collect = LivingEntity.class.getDeclaredMethod("collectEquipmentChanges", Map.class);
            collect.setAccessible(true);
            collect.invoke(ent, last);
        }
        return ent;
    }

    static void run(JsonObject f, List<String> result) throws Exception {
        VanillaBlockUpdateOracle.World w = VanillaBlockUpdateOracle.build(f);
        w.entities = new ArrayList<>();
        w.registries = ParityData.access;
        w.server.loot.delegate = ParityData.loot;
        if (Chunks.generator == null) Chunks.generator = VanillaBlockUpdateOracle.make(net.minecraft.world.level.levelgen.FlatLevelSource.class);
        VanillaBlockUpdateOracle.set(ServerLevel.class, w, "chunkSource", VanillaBlockUpdateOracle.make(Chunks.class));
        VanillaBlockUpdateOracle.set(Level.class, w, "damageSources", new net.minecraft.world.damagesource.DamageSources(ParityData.access));
        if (f.has("rules")) {
            for (var rule : f.getAsJsonObject("rules").entrySet()) {
                if (rule.getKey().equals("tnt_explodes")) w.rules.set(GameRules.TNT_EXPLODES, rule.getValue().getAsBoolean(), null);
            }
        }
        if (f.has("items")) {
            for (var item : f.getAsJsonArray("items")) {
                var it = item.getAsJsonArray();
                BlockPos at = new BlockPos(it.get(0).getAsInt(), it.get(1).getAsInt(), it.get(2).getAsInt());
                slots(w.getBlockEntity(at)).set(it.get(3).getAsInt(), stack(it.get(4).getAsString()));
            }
        }
        if (f.has("entities")) {
            List<Entity> created = new ArrayList<>();
            for (var e : f.getAsJsonArray("entities")) {
                Entity ent = target(w, e.getAsJsonObject());
                created.add(ent);
                w.entities.add(ent);
            }
            var list = f.getAsJsonArray("entities");
            for (int i = 0; i < list.size(); i++) {
                var e = list.get(i).getAsJsonObject();
                if (e.has("leash") && created.get(i) instanceof Leashable leashable) {
                    leashable.setLeashedTo(created.get(e.get("leash").getAsInt()), true);
                }
            }
        }
        // Creating the targets must not move the stream the dispenser draws from.
        w.rng = new LegacyRandomSource(f.get("seed").getAsLong());
        VanillaBlockUpdateOracle.set(Level.class, w, "random", w.rng);
        w.log = new StringBuilder();
        w.blockEvents.clear();
        Map<BlockPos, BlockState> before = new HashMap<>(w.cells);
        List<Entity> targets = List.copyOf(w.entities);
        BlockPos origin = VanillaBlockUpdateOracle.readPos(f.getAsJsonArray("pos"));
        BlockState state = w.getBlockState(origin);
        dispenseFrom(state.getBlock().getClass()).invoke(state.getBlock(), w, state, origin);

        result.addAll(VanillaBlockUpdateOracle.summary(w, before));
        List<BlockPos> containers = new ArrayList<>(w.blockEntities == null ? List.of() : w.blockEntities.keySet());
        containers.sort(Comparator.<BlockPos>comparingInt(q -> q.getX()).thenComparingInt(q -> q.getY()).thenComparingInt(q -> q.getZ()));
        for (BlockPos at : containers) {
            if (!(w.blockEntities.get(at) instanceof Container) || w.getBlockEntity(at) != w.blockEntities.get(at)) continue;
            var items = slots(w.blockEntities.get(at));
            for (int i = 0; i < items.size(); i++) {
                if (!items.get(i).isEmpty()) result.add("inv " + VanillaBlockUpdateOracle.pos(at) + " " + i + " " + ParityData.canon(items.get(i)));
            }
        }
        for (Entity e : w.entities) {
            if (targets.contains(e)) continue;
            StringBuilder line = new StringBuilder("N ").append(BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()));
            line.append(' ').append(num(e.getX())).append(' ').append(num(e.getY())).append(' ').append(num(e.getZ()));
            var v = e.getDeltaMovement();
            line.append(' ').append(num(v.x)).append(' ').append(num(v.y)).append(' ').append(num(v.z));
            if (e instanceof ItemEntity item) line.append(' ').append(ParityData.canon(item.getItem()));
            if (e instanceof PrimedTnt tnt) line.append(" fuse=").append(tnt.getFuse());
            result.add(line.toString());
        }
        for (int i = 0; i < targets.size(); i++) result.add(describe(i, targets.get(i)));
        for (String s : w.log.toString().split("\n")) {
            if (s.isEmpty() || s.startsWith("E ")) continue;
            result.add(s);
            if (s.startsWith("L ")) {
                String[] p = s.split(" ");
                String sound = eventSound(Integer.parseInt(p[1]), Integer.parseInt(p[3]));
                if (sound != null) result.add("S " + p[2] + " " + sound);
            }
        }
        result.addAll(w.blockEvents);
        result.add("rng " + w.rng.nextLong());
    }

    /** A target's state after dispensing: removal, equipment and readiness for shearing. */
    static String describe(int index, Entity e) {
        StringBuilder line = new StringBuilder("T " + index + " " + BuiltInRegistries.ENTITY_TYPE.getKey(e.getType()));
        if (e.isRemoved()) line.append(" removed");
        if (e instanceof LivingEntity living) {
            for (EquipmentSlot slot : EquipmentSlot.values()) {
                ItemStack s = living.getItemBySlot(slot);
                if (!s.isEmpty()) line.append(' ').append(slot.getName()).append('=').append(BuiltInRegistries.ITEM.getKey(s.getItem())).append('x').append(s.getCount());
            }
        }
        boolean listed = e instanceof net.minecraft.world.entity.animal.sheep.Sheep || e instanceof net.minecraft.world.entity.animal.cow.MushroomCow
            || e instanceof net.minecraft.world.entity.monster.skeleton.Bogged || e instanceof net.minecraft.world.entity.animal.golem.SnowGolem;
        if (listed && e instanceof Shearable shearable) line.append(shearable.readyForShearing() ? " ready" : " unready");
        if (e instanceof net.minecraft.world.entity.animal.equine.AbstractChestedHorse chested && chested.hasChest()) line.append(" chest");
        if (e instanceof Leashable leashable && leashable.isLeashed()) line.append(" leashed");
        return line.toString();
    }
}
