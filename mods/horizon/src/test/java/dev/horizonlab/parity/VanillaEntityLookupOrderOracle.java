package dev.horizonlab.parity;

import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.UUID;
import net.minecraft.world.level.entity.EntityAccess;
import net.minecraft.world.level.entity.EntityLookup;

/** Observe the real lookup, without constructing a world or entity subclasses. */
public final class VanillaEntityLookupOrderOracle {
    private static EntityAccess entity(int id) {
        return (EntityAccess) Proxy.newProxyInstance(EntityAccess.class.getClassLoader(),
            new Class<?>[] {EntityAccess.class}, (proxy, method, args) -> switch (method.getName()) {
                case "getId" -> id;
                case "getUUID" -> new UUID(0, id);
                case "toString" -> "lookup fixture " + id;
                default -> throw new UnsupportedOperationException(method.getName());
            });
    }
    private static void snapshot(EntityLookup<EntityAccess> lookup) {
        var ids = new ArrayList<Integer>();
        lookup.getAllEntities().forEach(e -> ids.add(e.getId()));
        System.out.println("lookup-order: " + ids);
    }
    public static void main(String[] args) {
        var home = new EntityLookup<EntityAccess>();
        var away = new EntityLookup<EntityAccess>();
        var first = entity(90);
        home.add(first);
        home.add(entity(3));
        home.add(entity(40));
        away.add(entity(1));
        snapshot(home);
        // A temporary engine storage loan does not change lookup membership.
        snapshot(home);
        home.remove(first);
        home.add(first);
        snapshot(home);
        home.remove(first);
        away.add(first);
        snapshot(home);
        snapshot(away);
        away.remove(first);
        home.add(first);
        snapshot(home);
    }
}
