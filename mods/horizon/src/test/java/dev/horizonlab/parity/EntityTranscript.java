package dev.horizonlab.parity;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientboundAddEntityPacket;
import net.minecraft.network.protocol.game.ClientboundEntityPositionSyncPacket;
import net.minecraft.network.protocol.game.ClientboundMoveEntityPacket;
import net.minecraft.network.protocol.game.ClientboundProjectilePowerPacket;
import net.minecraft.network.protocol.game.ClientboundRemoveEntitiesPacket;
import net.minecraft.network.protocol.game.ClientboundRotateHeadPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityDataPacket;
import net.minecraft.network.protocol.game.ClientboundSetEntityMotionPacket;
import net.minecraft.network.protocol.game.ClientboundSetEquipmentPacket;
import net.minecraft.network.protocol.game.ClientboundSetPassengersPacket;
import net.minecraft.network.protocol.game.ClientboundTeleportEntityPacket;
import net.minecraft.network.protocol.game.ClientboundUpdateAttributesPacket;
import net.minecraft.network.protocol.game.VecDeltaCodec;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.phys.Vec3;

/**
 * What a vanilla client knows about the entities a server tracks to it, as
 * ClientPacketListener would apply the packets: each entity's spawn packet,
 * the entity data it was paired with and all it was sent since, its
 * equipment, attributes and passengers, and where the position syncs leave
 * it (VecDeltaCodec, as handleMoveEntity and handleEntityPositionSync).
 * Entity ids differ between servers: an entity is named by its type and the
 * block it was added in, passengers by those names. Players are left out, but
 * for the attributes the player itself is sent from its login on.
 */
final class EntityTranscript {
    private static final class Tracked {
        final String type;
        final String add;
        final VecDeltaCodec codec = new VecDeltaCodec();
        String name;
        String pairing;
        final Map<Integer, String> data = new TreeMap<>();
        final Map<String, String> equipment = new TreeMap<>();
        final Map<String, String> attributes = new TreeMap<>();
        int[] passengers = new int[0];
        float yRot;
        float xRot;
        float headRot;
        boolean onGround;
        String motion;
        String power = "";
        boolean removed;

        Tracked(ClientboundAddEntityPacket p) {
            type = BuiltInRegistries.ENTITY_TYPE.getKey(p.getType()).toString();
            Vec3 at = new Vec3(p.getX(), p.getY(), p.getZ());
            codec.setBase(at);
            name = type + " " + (int) Math.floor(at.x) + "," + (int) Math.floor(at.y) + "," + (int) Math.floor(at.z);
            add = "at " + at + " rot " + p.getXRot() + "," + p.getYRot() + " head " + p.getYHeadRot() + " data " + p.getData()
                + " motion " + p.getMovement();
            yRot = p.getYRot();
            xRot = p.getXRot();
            headRot = p.getYHeadRot();
            motion = String.valueOf(p.getMovement());
        }
    }

    private final Map<Integer, Tracked> byId = new HashMap<>();
    private final List<Tracked> all = new ArrayList<>();
    private int self = Integer.MIN_VALUE;
    private final Map<String, String> selfAttributes = new TreeMap<>();

    /** The player's own id (login) and attribute instances; false for any other packet. */
    boolean acceptSelf(Packet<?> packet) {
        if (packet instanceof net.minecraft.network.protocol.game.ClientboundLoginPacket p) {
            self = p.playerId();
            return true;
        }
        if (packet instanceof ClientboundUpdateAttributesPacket p && p.getEntityId() == self) {
            p.getValues().forEach(a -> selfAttributes.put(a.attribute().getRegisteredName(), a.base() + " " + a.modifiers()));
            return true;
        }
        return false;
    }
    private int unknown;

    /** Applies one packet; false when it isn't about a tracked entity. */
    boolean accept(Packet<?> packet) {
        if (packet instanceof ClientboundAddEntityPacket p) {
            if (p.getType() == EntityTypes.PLAYER) return true;
            Tracked t = new Tracked(p);
            byId.put(p.getId(), t);
            all.add(t);
            return true;
        }
        if (packet instanceof ClientboundRemoveEntitiesPacket p) {
            p.entityIds().forEach(id -> {
                Tracked t = byId.remove(id);
                if (t != null) t.removed = true;
            });
            return true;
        }
        Tracked t = switch (packet) {
            case ClientboundSetEntityDataPacket p -> byId.get(p.id());
            case ClientboundSetEquipmentPacket p -> byId.get(p.getEntity());
            case ClientboundUpdateAttributesPacket p -> byId.get(p.getEntityId());
            case ClientboundSetPassengersPacket p -> byId.get(p.getVehicle());
            case ClientboundEntityPositionSyncPacket p -> byId.get(p.id());
            case ClientboundSetEntityMotionPacket p -> byId.get(p.id());
            case ClientboundTeleportEntityPacket p -> byId.get(p.id());
            case ClientboundProjectilePowerPacket p -> byId.get(p.getId());
            case ClientboundMoveEntityPacket p -> byId.get(VanillaProtocolClient.moveEntityId(p));
            case ClientboundRotateHeadPacket p -> byId.get(VanillaProtocolClient.rotateHeadId(p));
            default -> null;
        };
        if (t == null) {
            return false;
        }
        switch (packet) {
            case ClientboundSetEntityDataPacket p -> {
                for (var v : p.packedItems()) t.data.put(v.id(), net.minecraft.network.syncher.EntityDataSerializers.getSerializedId(v.serializer()) + " " + v.value());
                if (t.pairing == null) {
                    StringBuilder b = new StringBuilder();
                    for (var v : p.packedItems()) b.append(v.id()).append('=').append(v.value()).append(' ');
                    t.pairing = b.toString().trim();
                }
            }
            case ClientboundSetEquipmentPacket p -> p.getSlots().forEach(s -> t.equipment.put(s.getFirst().getName(), String.valueOf(s.getSecond())));
            case ClientboundUpdateAttributesPacket p -> p.getValues().forEach(a -> t.attributes.put(a.attribute().getRegisteredName(),
                a.base() + " " + a.modifiers()));
            case ClientboundSetPassengersPacket p -> t.passengers = p.getPassengers();
            case ClientboundEntityPositionSyncPacket p -> {
                t.codec.setBase(p.position().endPosition());
                t.yRot = p.yRot();
                t.xRot = p.xRot();
                t.onGround = p.onGround();
            }
            case ClientboundMoveEntityPacket p -> {
                if (p.hasPosition()) t.codec.setBase(p.getPositionDelta().decode(t.codec).endPosition());
                if (p.hasRotation()) {
                    t.yRot = p.getYRot();
                    t.xRot = p.getXRot();
                }
                t.onGround = p.isOnGround();
            }
            case ClientboundSetEntityMotionPacket p -> t.motion = String.valueOf(p.movement());
            case ClientboundTeleportEntityPacket p -> {
                t.codec.setBase(p.change().position());
                t.onGround = p.onGround();
            }
            case ClientboundRotateHeadPacket p -> t.headRot = p.getYHeadRot();
            case ClientboundProjectilePowerPacket p -> t.power = " power " + p.getAccelerationPower();
            default -> {
            }
        }
        return true;
    }

    /** One line per entity, sorted, each field named. */
    List<String> lines() {
        Map<String, Integer> seen = new HashMap<>();
        for (Tracked t : all) {
            int n = seen.merge(t.name, 1, Integer::sum);
            if (n > 1) t.name += " #" + n;
        }
        Map<Integer, String> names = new HashMap<>();
        byId.forEach((id, t) -> names.put(id, t.name));
        List<String> out = new ArrayList<>();
        for (Tracked t : all) {
            List<String> riders = new ArrayList<>();
            for (int id : t.passengers) riders.add(names.getOrDefault(id, "player or unknown"));
            out.add("entity " + t.name + " | add " + t.add + " | pairing " + t.pairing + " | data " + t.data + " | equipment " + t.equipment
                + " | attributes " + t.attributes + " | passengers " + riders + " | now " + t.codec.getBase() + " rot " + t.xRot + ","
                + t.yRot + " head " + t.headRot + " ground " + t.onGround + " motion " + t.motion + t.power + (t.removed ? " | removed" : ""));
        }
        out.add("self attributes " + selfAttributes);
        out.sort(null);
        return out;
    }
}
