package dev.horizonlab.parity;

import java.io.BufferedWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Predicate;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.multiplayer.prediction.BlockStatePredictionHandler;
import net.minecraft.client.player.ClientInput;
import net.minecraft.client.player.FirstPersonHandsAndItems;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.network.protocol.Packet;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Marker;
import net.minecraft.world.entity.ai.attributes.AttributeMap;
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.entity.player.Abilities;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.entity.player.PlayerEquipment;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemCooldowns;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

/**
 * Calls vanilla LocalPlayer.pick against a fixture level with real entity bounds and block shapes. Then, with a
 * wind charge behind and in front of the TNT, hands that pick to the real Minecraft.startUseItem (a stick in the
 * main hand, the off hand empty) and prints the packets MultiPlayerGameMode sends: the level, the connection and
 * the player are stand-ins, the click runs the game's code.
 */
public final class VanillaPrimedTntCrosshairOracle {
    private final Object unsafe;
    private final Method allocate;

    private VanillaPrimedTntCrosshairOracle() throws Exception {
        var type = Class.forName("sun.misc.Unsafe");
        unsafe = field(type, "theUnsafe").get(null);
        allocate = type.getMethod("allocateInstance", Class.class);
    }

    private static Field field(Class<?> owner, String name) throws Exception {
        for (Class<?> type = owner; type != null; type = type.getSuperclass()) {
            try {
                var field = type.getDeclaredField(name);
                field.setAccessible(true);
                return field;
            } catch (NoSuchFieldException e) {
                // try the superclass
            }
        }
        throw new NoSuchFieldException(owner + "." + name);
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

        @Override
        public FeatureFlagSet enabledFeatures() {
            return FeatureFlags.REGISTRY.allFlags();
        }

        @Override
        public boolean isClientSide() {
            return true;
        }
    }

    /** Keeps what the click hands to the connection, in order. */
    static final class Wire extends ClientPacketListener {
        List<Packet<?>> sent;

        Wire() {
            super(null, null, null);
        }

        @Override
        public void send(Packet<?> packet) {
            sent.add(packet);
        }
    }

    static final class Clicker extends LocalPlayer {
        AttributeMap attributes;

        Clicker() {
            super(null, null, null, null, null, null, false, null, null);
        }

        @Override
        public AttributeMap getAttributes() {
            return attributes;
        }

        // The connection shell knows no players: the game mode is answered here.
        @Override
        public boolean isSpectator() {
            return false;
        }

        @Override
        public boolean isCreative() {
            return false;
        }

        // The shell Minecraft has no game renderer: the player is the camera.
        @Override
        protected boolean isControlledCamera() {
            return true;
        }

        @Override
        public boolean onGround() {
            return true;
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
            useClicks(writer, level, camera, pick);
        }
    }

    /** One use click for each order of a wind charge and the TNT along the view: the pick, then the packets. */
    private void useClicks(BufferedWriter writer, FixtureLevel level, Entity camera, Method pick) throws Exception {
        field(ClientLevel.class, "worldBorder").set(level, new WorldBorder());
        field(ClientLevel.class, "blockStatePredictionHandler").set(level, new BlockStatePredictionHandler());
        level.wall = false;
        var minecraft = create(Minecraft.class);
        // What reads Minecraft.getInstance() meets the same shell.
        field(Minecraft.class, "instance").set(null, minecraft);
        var wire = create(Wire.class);
        wire.sent = new ArrayList<>();
        var player = create(Clicker.class);
        field(LocalPlayer.class, "minecraft").set(player, minecraft);
        field(LocalPlayer.class, "connection").set(player, wire);
        field(Entity.class, "level").set(player, level);
        field(Entity.class, "id").setInt(player, 1);
        field(Entity.class, "position").set(player, camera.position());
        var equipment = new PlayerEquipment(player);
        field(LivingEntity.class, "equipment").set(player, equipment);
        var inventory = new Inventory(player, equipment);
        field(Player.class, "inventory").set(player, inventory);
        field(Player.class, "abilities").set(player, new Abilities());
        field(Player.class, "cooldowns").set(player, new ItemCooldowns());
        field(Player.class, "lastItemInMainHand").set(player, ItemStack.EMPTY);
        field(LivingEntity.class, "swingState").set(player, new LivingEntity.SwingState());
        field(LocalPlayer.class, "firstPersonHandsAndItems").set(player, new FirstPersonHandsAndItems());
        player.input = new ClientInput();
        player.attributes = new AttributeMap(Player.createAttributes().build());
        inventory.setItem(0, new ItemStack(Items.STICK));
        var gameMode = new MultiPlayerGameMode(minecraft, wire);
        field(MultiPlayerGameMode.class, "localPlayerMode").set(gameMode, GameType.SURVIVAL);
        field(Minecraft.class, "player").set(minecraft, player);
        field(Minecraft.class, "level").set(minecraft, level);
        field(Minecraft.class, "gameMode").set(minecraft, gameMode);
        var startUseItem = Minecraft.class.getDeclaredMethod("startUseItem");
        startUseItem.setAccessible(true);
        var chargeType = (EntityType<?>) EntityTypes.class.getField("WIND_CHARGE").get(null);
        var chargeClass = Class.forName(
            "net.minecraft.world.entity.projectile.hurtingprojectile.windcharge.WindCharge").asSubclass(Entity.class);
        for (String order : List.of("tnt_first", "charge_first")) {
            var tnt = placed(entity(PrimedTnt.class, EntityTypes.TNT, level, new Vec3(0.5, 65, 5.5)), 11);
            double z = order.equals("tnt_first") ? 7.35 : 5.0;
            var charge = placed(entity(chargeClass, chargeType, level, new Vec3(0.5, 65.5, z)), 12);
            level.entities = List.of(tnt, charge);
            var hit = (HitResult) pick.invoke(null, camera, 4.5, 3.0, 1.0f);
            int picked = hit instanceof EntityHitResult entityHit ? entityHit.getEntity().getId() : 0;
            writer.write("pick " + order + " " + name(picked));
            writer.newLine();
            field(Minecraft.class, "hitResult").set(minecraft, hit);
            wire.sent.clear();
            startUseItem.invoke(minecraft);
            for (var packet : wire.sent) {
                writer.write("use " + order + " " + describe(packet));
                writer.newLine();
            }
        }
    }

    private static <T extends Entity> T placed(T entity, int id) throws Exception {
        field(Entity.class, "id").setInt(entity, id);
        field(Entity.class, "blockPosition").set(entity, BlockPos.containing(entity.position()));
        return entity;
    }

    private static String name(int id) {
        return switch (id) {
            case 0 -> "none";
            case 11 -> "tnt";
            case 12 -> "wind_charge";
            default -> "other";
        };
    }

    /** A packet as its kind, hand and target entity, read from its own fields. */
    private static String describe(Packet<?> packet) throws Exception {
        InteractionHand hand = null;
        Integer first = null;
        for (var field : packet.getClass().getDeclaredFields()) {
            if (Modifier.isStatic(field.getModifiers())) {
                continue;
            }
            field.setAccessible(true);
            if (field.getType() == InteractionHand.class) {
                hand = (InteractionHand) field.get(packet);
            } else if (field.getType() == int.class && first == null) {
                first = field.getInt(packet);
            }
        }
        String side = hand == null ? "?" : hand == InteractionHand.MAIN_HAND ? "main" : "off";
        return switch (packet.getClass().getSimpleName()) {
            case "ServerboundInteractPacket" -> "interact " + side + " " + (first == null ? "?" : name(first));
            case "ServerboundUseItemPacket" -> "use_item " + side;
            default -> "other " + packet.getClass().getSimpleName();
        };
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        new VanillaPrimedTntCrosshairOracle().run(Path.of(args[0]));
    }
}
