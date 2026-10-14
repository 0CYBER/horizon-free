package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.client.multiplayer.prediction.BlockStatePredictionHandler;
import net.minecraft.client.player.FirstPersonHandsAndItems;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.client.tutorial.Tutorial;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.protocol.Packet;
import net.minecraft.resources.Identifier;
import net.minecraft.tags.TagKey;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.effect.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.ai.attributes.*;
import net.minecraft.world.entity.player.*;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.component.SwingAnimation;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.level.gameevent.GameEvent;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.*;

/**
 * Drives the real Minecraft.startAttack, continueAttack and startUseItem,
 * MultiPlayerGameMode's startDestroyBlock, continueDestroyBlock,
 * stopDestroyBlock, destroyBlock and attack, LivingEntity.swing with its
 * SwingState and getModifiedSwingDuration, Player's attack strength and item
 * swap tickers and getItemSwapScale, and FirstPersonHandsAndItems.tick and
 * itemUsed, tick by tick over scripted clicks and crosshair targets. The
 * player is a LocalPlayer shell with a real Inventory and attributes; the
 * level, connection and item model resolver are shells answering what these
 * read, and the results of using items on blocks, entities and the air are
 * scripted. The attack and use part of Minecraft.handleKeybinds and the
 * ticker lines of Player.tick are mirrored here in vanilla's order. Steps may
 * turn the player; the real LocalPlayer.applyInput moves the hands' xBob and
 * yBob after it.
 */
public final class VanillaFirstPersonOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static Field field(Class<?> type, String name) throws Exception {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // try the superclass
            }
        }
        throw new NoSuchFieldException(type + "." + name);
    }

    static Method method(Class<?> type, String name, Class<?>... args) throws Exception {
        Method m = type.getDeclaredMethod(name, args);
        m.setAccessible(true);
        return m;
    }

    static String fb(float v) {
        return String.format("%08x", Float.floatToRawIntBits(v));
    }

    static List<String> events = new ArrayList<>();
    static Map<BlockPos, BlockState> blocks = new HashMap<>();
    static JsonObject step;

    static String hand(InteractionHand h) {
        return h == InteractionHand.MAIN_HAND ? "main" : "off";
    }

    static InteractionResult result(String name) {
        return switch (name) {
            case "success" -> InteractionResult.SUCCESS;
            case "server" -> InteractionResult.SUCCESS_SERVER;
            case "consume" -> InteractionResult.CONSUME;
            case "fail" -> InteractionResult.FAIL;
            default -> InteractionResult.PASS;
        };
    }

    static final class Level0 extends ClientLevel {
        Level0() {
            super(null, null, null, null, 0, 0, null, false, 0L, 0);
        }

        @Override public BlockState getBlockState(BlockPos pos) { return blocks.getOrDefault(pos, Blocks.AIR.defaultBlockState()); }
        @Override public FluidState getFluidState(BlockPos pos) { return Fluids.EMPTY.defaultFluidState(); }
        @Override public boolean setBlock(BlockPos pos, BlockState state, int flags, int limit) {
            blocks.put(pos.immutable(), state);
            return true;
        }
        @Override public void levelEvent(Entity source, int type, BlockPos pos, int data) { }
        @Override public void gameEvent(Holder<GameEvent> event, Vec3 pos, GameEvent.Context context) { }
        @Override public void destroyBlockProgress(int id, BlockPos pos, int progress) {
            events.add("stage " + pos.getX() + " " + pos.getY() + " " + pos.getZ() + " " + progress);
        }
        @Override public void addBreakingBlockEffects(BlockPos pos, Direction direction, boolean sound) { }
        @Override public FeatureFlagSet enabledFeatures() { return FeatureFlags.REGISTRY.allFlags(); }
    }

    static final class Connection0 extends ClientPacketListener {
        Connection0() {
            super(null, null, null);
        }

        @Override public void send(Packet<?> packet) { }
    }

    static final class Resolver extends ItemModelResolver {
        Resolver() {
            super(null);
        }

        // No vanilla item model turns hand_animation_on_swap off, and only
        // spears (not scripted here) scale the swap animation.
        @Override public boolean shouldPlaySwapAnimation(ItemStack stack) { return true; }
        @Override public float swapAnimationScale(ItemStack stack) { return 1.0F; }
    }

    static final class Player0 extends LocalPlayer {
        AttributeMap attributes;
        boolean inRange;

        Player0() {
            super(null, null, null, null, null, Input.EMPTY, false, null, null);
        }

        @Override public AttributeMap getAttributes() { return attributes; }
        // The shell Minecraft has no game renderer: the player is the camera.
        @Override protected boolean isControlledCamera() { return true; }
        @Override public boolean isEyeInFluid(TagKey<Fluid> type) { return false; }
        @Override public boolean onGround() { return true; }
        @Override public boolean isWithinEntityInteractionRange(Entity entity, double buffer) { return inRange; }
        @Override public void attack(Entity entity) { events.add("attack"); }
        @Override public boolean swing(InteractionHand hand, SwingAnimation animation, boolean send) {
            boolean started = super.swing(hand, animation, send);
            events.add("swing " + hand(hand) + " " + animation.duration() + " " + started);
            return started;
        }
        @Override public void itemUsed(InteractionHand hand) {
            events.add("used " + hand(hand));
            super.itemUsed(hand);
        }
    }

    static final class GameMode0 extends MultiPlayerGameMode {
        GameMode0(Minecraft minecraft, ClientPacketListener connection) {
            super(minecraft, connection);
        }

        @Override public boolean destroyBlock(BlockPos pos) {
            boolean changed = super.destroyBlock(pos);
            events.add("destroy " + pos.getX() + " " + pos.getY() + " " + pos.getZ() + " " + changed);
            return changed;
        }
        @Override public void releaseUsingItem(Player player) { events.add("release"); }

        InteractionResult scripted(String kind, Player player, InteractionHand hand) {
            String name = step.getAsJsonObject("uses").get(kind).getAsString();
            events.add("use " + kind + " " + hand(hand) + " " + name);
            InteractionResult r = result(name);
            if (r instanceof InteractionResult.Success && step.get("shrink").getAsBoolean()) {
                player.getItemInHand(hand).shrink(1);
            }
            return r;
        }

        @Override public InteractionResult useItemOn(LocalPlayer player, InteractionHand hand, BlockHitResult hit) { return scripted("block", player, hand); }
        @Override public InteractionResult useItem(Player player, InteractionHand hand) { return scripted("item", player, hand); }
        @Override public InteractionResult interact(Player player, Entity entity, EntityHitResult hit, InteractionHand hand) {
            return scripted("entity", player, hand);
        }
    }

    static ItemStack stack(JsonElement e) {
        if (e.isJsonNull()) {
            return ItemStack.EMPTY;
        }
        var a = e.getAsJsonArray();
        return new ItemStack(BuiltInRegistries.ITEM.getValue(Identifier.parse(a.get(0).getAsString())), a.get(1).getAsInt());
    }

    /** LivingEntity.collectEquipmentChanges for the main hand: the item's attribute modifiers. */
    static AttributeMap attributes(ItemStack main) {
        var map = new AttributeMap(Player.createAttributes().build());
        main.forEachModifier(EquipmentSlot.MAINHAND, (attribute, modifier) -> {
            var instance = map.getInstance(attribute);
            if (instance != null) {
                instance.addTransientModifier(modifier);
            }
        });
        return map;
    }

    public static void main(String[] args) throws Exception {
        // The vanilla data pack: item components (tools, attribute modifiers) and block tags.
        ParityData.init();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        Method startAttack = method(Minecraft.class, "startAttack");
        Method continueAttack = method(Minecraft.class, "continueAttack", boolean.class);
        Method startUseItem = method(Minecraft.class, "startUseItem");
        Field missTime = field(Minecraft.class, "missTime");
        Field rightClickDelay = field(Minecraft.class, "rightClickDelay");
        Field swingState = field(LivingEntity.class, "swingState");
        Field attackTicker = field(LivingEntity.class, "attackStrengthTicker");
        Field swapTicker = field(LivingEntity.class, "itemSwapTicker");
        Field lastItem = field(Player.class, "lastItemInMainHand");
        Field hands = field(LocalPlayer.class, "firstPersonHandsAndItems");
        Field handsBusy = field(LocalPlayer.class, "handsBusy");
        Field using = field(LocalPlayer.class, "startedUsingItem");
        Field destroyDelay = field(MultiPlayerGameMode.class, "destroyDelay");
        Field destroying = field(MultiPlayerGameMode.class, "isDestroying");
        Field progress = field(MultiPlayerGameMode.class, "destroyProgress");
        Field mainHeight = field(FirstPersonHandsAndItems.class, "mainHandHeight");
        Field oMainHeight = field(FirstPersonHandsAndItems.class, "oMainHandHeight");
        Field offHeight = field(FirstPersonHandsAndItems.class, "offHandHeight");
        Field oOffHeight = field(FirstPersonHandsAndItems.class, "oOffHandHeight");
        Field ticks = field(LivingEntity.SwingState.class, "ticks");
        Field animation = field(LivingEntity.SwingState.class, "animation");
        Field oldAnimation = field(LivingEntity.SwingState.class, "oldAnimation");
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            int n = 0;
            for (var line : (Iterable<String>) lines::iterator) {
                var c = JsonParser.parseString(line).getAsJsonObject();
                out.println("== " + n++);
                blocks.clear();
                for (var e : c.getAsJsonArray("blocks")) {
                    var b = e.getAsJsonArray();
                    blocks.put(new BlockPos(b.get(0).getAsInt(), b.get(1).getAsInt(), b.get(2).getAsInt()),
                        BuiltInRegistries.BLOCK.getValue(Identifier.parse(b.get(3).getAsString())).defaultBlockState());
                }
                Minecraft mc = make(Minecraft.class);
                Level0 level = make(Level0.class);
                field(ClientLevel.class, "worldBorder").set(level, new WorldBorder());
                field(ClientLevel.class, "blockStatePredictionHandler").set(level, new BlockStatePredictionHandler());
                Connection0 connection = make(Connection0.class);
                Player0 player = make(Player0.class);
                field(LocalPlayer.class, "minecraft").set(player, mc);
                field(LocalPlayer.class, "connection").set(player, connection);
                field(Entity.class, "level").set(player, level);
                field(Entity.class, "id").setInt(player, 1);
                var equipment = new PlayerEquipment(player);
                field(LivingEntity.class, "equipment").set(player, equipment);
                field(Player.class, "inventory").set(player, new Inventory(player, equipment));
                var abilities = new Abilities();
                boolean creative = c.get("creative").getAsBoolean();
                abilities.instabuild = creative;
                field(Player.class, "abilities").set(player, abilities);
                swingState.set(player, new LivingEntity.SwingState());
                lastItem.set(player, ItemStack.EMPTY);
                hands.set(player, new FirstPersonHandsAndItems());
                player.input = new net.minecraft.client.player.ClientInput();
                var effects = new HashMap<Holder<MobEffect>, MobEffectInstance>();
                if (!c.get("haste").isJsonNull()) {
                    effects.put(MobEffects.HASTE, new MobEffectInstance(MobEffects.HASTE, 100000, c.get("haste").getAsInt()));
                }
                if (!c.get("fatigue").isJsonNull()) {
                    effects.put(MobEffects.MINING_FATIGUE, new MobEffectInstance(MobEffects.MINING_FATIGUE, 100000, c.get("fatigue").getAsInt()));
                }
                field(LivingEntity.class, "activeEffects").set(player, effects);
                player.getInventory().setItem(0, stack(c.get("main")));
                equipment.set(EquipmentSlot.OFFHAND, stack(c.get("off")));
                player.attributes = attributes(player.getMainHandItem());
                field(Minecraft.class, "player").set(mc, player);
                field(Minecraft.class, "level").set(mc, level);
                field(Minecraft.class, "tutorial").set(mc, make(Tutorial.class));
                field(Minecraft.class, "itemModelResolver").set(mc, make(Resolver.class));
                var gameMode = new GameMode0(mc, connection);
                field(MultiPlayerGameMode.class, "localPlayerMode").set(gameMode, creative ? GameType.CREATIVE : GameType.SURVIVAL);
                field(Minecraft.class, "gameMode").set(mc, gameMode);
                out.println("attack_speed " + Long.toHexString(Double.doubleToRawLongBits(player.getAttributeValue(Attributes.ATTACK_SPEED))));
                var entity = make(net.minecraft.world.entity.item.ItemEntity.class);
                field(Entity.class, "blockPosition").set(entity, BlockPos.ZERO);
                field(Entity.class, "id").setInt(entity, 2);
                int s = 0;
                for (var e : c.getAsJsonArray("steps")) {
                    step = e.getAsJsonObject();
                    events.clear();
                    if (!step.get("set").isJsonNull()) {
                        var set = step.getAsJsonObject("set");
                        if (set.get("hand").getAsString().equals("main")) {
                            player.getInventory().setItem(0, stack(set.get("stack")));
                        } else {
                            equipment.set(EquipmentSlot.OFFHAND, stack(set.get("stack")));
                        }
                    }
                    var hit = step.getAsJsonArray("hit");
                    HitResult hitResult = switch (hit.get(0).getAsString()) {
                        case "block" -> new BlockHitResult(Vec3.ZERO, Direction.from3DDataValue(hit.get(4).getAsInt()),
                            new BlockPos(hit.get(1).getAsInt(), hit.get(2).getAsInt(), hit.get(3).getAsInt()), false);
                        case "entity" -> {
                            player.inRange = hit.get(1).getAsBoolean();
                            yield new EntityHitResult(entity, Vec3.ZERO);
                        }
                        default -> BlockHitResult.miss(Vec3.ZERO, Direction.UP, BlockPos.ZERO);
                    };
                    field(Minecraft.class, "hitResult").set(mc, hitResult);
                    handsBusy.setBoolean(player, step.get("busy").getAsBoolean());
                    using.setBoolean(player, step.get("using").getAsBoolean());
                    boolean attackDown = step.get("ad").getAsBoolean();
                    boolean useDown = step.get("ud").getAsBoolean();
                    // Minecraft.tick: rightClickDelay, then handleKeybinds' attack and use part.
                    if (rightClickDelay.getInt(mc) > 0) {
                        rightClickDelay.setInt(mc, rightClickDelay.getInt(mc) - 1);
                    }
                    boolean instant = false;
                    if (player.isUsingItem()) {
                        if (!useDown) {
                            gameMode.releaseUsingItem(player);
                        }
                    } else {
                        for (int i = 0; i < step.get("ac").getAsInt(); i++) {
                            instant |= (boolean) startAttack.invoke(mc);
                        }
                        for (int i = 0; i < step.get("uc").getAsInt(); i++) {
                            startUseItem.invoke(mc);
                        }
                    }
                    if (useDown && rightClickDelay.getInt(mc) == 0 && !player.isUsingItem()) {
                        startUseItem.invoke(mc);
                    }
                    continueAttack.invoke(mc, !instant && attackDown);
                    if (missTime.getInt(mc) > 0) {
                        missTime.setInt(mc, missTime.getInt(mc) - 1);
                    }
                    // LocalPlayer.tick: LivingEntity.baseTick's swing, the equipment's
                    // attribute modifiers, Player.tick's tickers, then
                    // FirstPersonHandsAndItems.tick.
                    var swing = (LivingEntity.SwingState) swingState.get(player);
                    swing.tick();
                    // LivingEntity.tick's detectEquipmentUpdates: the main hand's modifiers.
                    player.attributes = attributes(player.getMainHandItem());
                    attackTicker.setInt(player, attackTicker.getInt(player) + 1);
                    swapTicker.setInt(player, swapTicker.getInt(player) + 1);
                    ItemStack main = player.getMainHandItem();
                    if (!ItemStack.matches((ItemStack) lastItem.get(player), main)) {
                        if (!ItemStack.isSameItem((ItemStack) lastItem.get(player), main)) {
                            player.resetAttackStrengthTicker();
                        }
                        lastItem.set(player, main.copy());
                    }
                    // LocalPlayer.aiStep's applyInput, after the step's turn.
                    if (step.has("rot") && !step.get("rot").isJsonNull()) {
                        var rot = step.getAsJsonArray("rot");
                        player.setXRot(Float.intBitsToFloat(Integer.parseUnsignedInt(rot.get(0).getAsString(), 16)));
                        player.setYRot(Float.intBitsToFloat(Integer.parseUnsignedInt(rot.get(1).getAsString(), 16)));
                    }
                    player.applyInput();
                    var fp = (FirstPersonHandsAndItems) hands.get(player);
                    fp.tick(player);
                    for (var ev : events) {
                        out.println(ev);
                    }
                    var current = player.getCurrentSwing();
                    out.println("tick " + s++ + " " + (current == null ? "- 0" : hand(current.hand()) + " " + current.durationTicks())
                        + " " + ticks.getInt(swing) + " " + fb(animation.getFloat(swing)) + " " + fb(oldAnimation.getFloat(swing))
                        + " " + fb(player.getSwingAnimation(0.5F))
                        + " " + attackTicker.getInt(player) + " " + swapTicker.getInt(player) + " " + fb(player.getItemSwapScale(1.0F))
                        + " " + fb(mainHeight.getFloat(fp)) + " " + fb(oMainHeight.getFloat(fp))
                        + " " + fb(offHeight.getFloat(fp)) + " " + fb(oOffHeight.getFloat(fp))
                        + " " + missTime.getInt(mc) + " " + rightClickDelay.getInt(mc) + " " + destroyDelay.getInt(gameMode)
                        + " " + destroying.getBoolean(gameMode) + " " + fb(progress.getFloat(gameMode))
                        + " " + fb(player.xBob) + " " + fb(player.yBob) + " " + fb(player.xBobO) + " " + fb(player.yBobO));
                }
            }
        }
        System.exit(0);
    }
}
