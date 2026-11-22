package dev.horizonlab.parity;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.function.Predicate;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.Marker;
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.Vec3;

/** Calls vanilla LocalPlayer.pick against a fixture level with real entity bounds and block shapes. */
public final class VanillaPrimedTntCrosshairOracle {
    private final Object unsafe;
    private final Method allocate;

    private VanillaPrimedTntCrosshairOracle() throws Exception {
        var type = Class.forName("sun.misc.Unsafe");
        unsafe = field(type, "theUnsafe").get(null);
        allocate = type.getMethod("allocateInstance", Class.class);
    }

    private static Field field(Class<?> owner, String name) throws Exception {
        var field = owner.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    private <T> T create(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }

    private <T extends Entity> T entity(Class<T> cls, EntityType<?> type, FixtureLevel level, Vec3 pos)
            throws Exception {
        T entity = create(cls);
        field(Entity.class, "type").set(entity, type);
        field(Entity.class, "level").set(entity, level);
        field(Entity.class, "position").set(entity, pos);
        entity.setBoundingBox(type.getDimensions().makeBoundingBox(pos));
        return entity;
    }

    static final class FixtureLevel extends ClientLevel {
        List<Entity> entities;
        boolean wall;

        FixtureLevel() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override
        public List<Entity> getEntities(Entity except, AABB box, Predicate<? super Entity> test) {
            return entities.stream()
                .filter(entity -> entity != except && entity.getBoundingBox().intersects(box))
                .filter(test)
                .toList();
        }

        @Override
        public BlockState getBlockState(BlockPos pos) {
            return (wall && pos.getX() == 0 && pos.getY() == 65 && pos.getZ() == 4
                ? Blocks.STONE : Blocks.AIR).defaultBlockState();
        }

        @Override
        public FluidState getFluidState(BlockPos pos) {
            return Fluids.EMPTY.defaultFluidState();
        }
    }

    static final class Camera extends Marker {
        Camera() {
            super(null, null);
        }

        @Override
        public boolean isDescending() {
            return false;
        }
    }

    private void run(Path output) throws Exception {
        var level = create(FixtureLevel.class);
        var camera = entity(Camera.class, EntityTypes.MARKER, level, new Vec3(0.5, 64, 3.5));
        camera.setBoundingBox(EntityTypes.PLAYER.getDimensions().makeBoundingBox(camera.position()));
        field(Entity.class, "eyeHeight").setFloat(camera, 1.62f);
        var pick = LocalPlayer.class.getDeclaredMethod("pick", Entity.class, double.class, double.class, float.class);
        pick.setAccessible(true);
        try (var writer = Files.newBufferedWriter(output)) {
            for (String name : List.of("open", "wall", "nearer_cow", "farther_cow", "out_of_reach", "removed")) {
                double z = name.equals("out_of_reach") ? 7.5 : 5.5;
                var tnt = entity(PrimedTnt.class, EntityTypes.TNT, level, new Vec3(0.5, 65, z));
                level.wall = name.equals("wall");
                level.entities = switch (name) {
                    case "nearer_cow" -> List.of(tnt, entity(Cow.class, EntityTypes.COW, level, new Vec3(0.5, 65, 4.5)));
                    case "farther_cow" -> List.of(tnt, entity(Cow.class, EntityTypes.COW, level, new Vec3(0.5, 65, 6.5)));
                    default -> List.of(tnt);
                };
                if (name.equals("removed")) {
                    field(Entity.class, "removalReason").set(tnt, Entity.RemovalReason.DISCARDED);
                }
                var hit = pick.invoke(null, camera, 4.5, 3.0, 1.0f);
                boolean picked = hit instanceof EntityHitResult entityHit && entityHit.getEntity() == tnt;
                writer.write(name + " " + picked);
                writer.newLine();
            }
        }
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        new VanillaPrimedTntCrosshairOracle().run(Path.of(args[0]));
    }
}
