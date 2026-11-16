package dev.horizonlab.parity;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.decoration.Cushion;
import net.minecraft.world.level.entity.EntityLookup;
import net.minecraft.world.level.entity.EntitySectionStorage;
import net.minecraft.world.level.entity.LevelEntityGetter;
import net.minecraft.world.level.entity.LevelEntityGetterAdapter;
import net.minecraft.world.level.entity.Visibility;

/** Real ServerLevel.getEntity and Cushion; only world construction and membership are fixture boundaries. */
public final class VanillaEntityLookupOracle {
    static final class World extends ServerLevel {
        EntityLookup<Entity> lookup;
        LevelEntityGetter<Entity> getter;
        World() { super(null, null, null, null, null, null, false, 0L, List.of(), false); }
        @Override public int getNextEntityId() { return 317; }
        @Override protected LevelEntityGetter<Entity> getEntities() { return getter; }
    }

    static World world() throws Exception {
        // Same constructor-free level fixture as the other entity oracles; no lookup method is stubbed.
        var type = Class.forName("sun.misc.Unsafe");
        var field = type.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        var world = (World) type.getMethod("allocateInstance", Class.class).invoke(field.get(null), World.class);
        world.lookup = new EntityLookup<>();
        world.getter = new LevelEntityGetterAdapter<>(world.lookup,
            new EntitySectionStorage<>(Entity.class, section -> Visibility.TRACKED));
        return world;
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var first = world();
        var second = world();
        var cushion = new Cushion(EntityTypes.CUSHION, first);
        cushion.setId(317);
        var result = new java.util.ArrayList<String>();
        result.add("absent " + (first.getEntity(317) == null));
        first.lookup.add(cushion);
        result.add("cushion " + (first.getEntity(317) == cushion));
        result.add("zero " + (first.getEntity(0) == null));
        result.add("unknown " + (first.getEntity(318) == null));
        result.add("other_level " + (second.getEntity(317) == null));
        first.lookup.remove(cushion);
        result.add("removed " + (first.getEntity(317) == null));
        second.lookup.add(cushion);
        result.add("transferred " + (second.getEntity(317) == cushion));
        result.add("old_level " + (first.getEntity(317) == null));
        Files.write(Path.of(args[0]), result);
    }
}
