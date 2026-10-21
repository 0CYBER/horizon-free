package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.ServerGamePacketListenerImpl;
import net.minecraft.world.Container;
import net.minecraft.world.SimpleContainer;
import net.minecraft.world.entity.EntityEquipment;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.inventory.*;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.crafting.RecipeManager;

/**
 * Original oracle: AbstractContainerMenu.clicked over vanilla's real menu
 * classes. A stand-in player (inventory, drops recorded) and a stand-in server
 * level (real RecipeManager, no networking) run each case's clicks; after
 * every click the slots, carried stack, data slots, drops and player state
 * are printed as canonical NBT.
 */
public final class VanillaMenuOracle {
    static Object unsafe;
    static java.lang.reflect.Method allocate;
    static RecipeManager recipes;
    static final List<String> drops = new ArrayList<>();
    /** onEquipItem calls that change the item (its sound and game event): slot and new stack. */
    static final List<String> equips = new ArrayList<>();
    /** The block entity behind a "crafter_be" menu (slot state packets go to it). */
    static net.minecraft.world.level.block.entity.CrafterBlockEntity crafter;

    @SuppressWarnings("unchecked")
    static <T> T make(Class<T> type) {
        try {
            return (T) allocate.invoke(unsafe, type);
        } catch (Exception e) {
            throw new AssertionError(e);
        }
    }

    static void set(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    static final class Server extends net.minecraft.server.dedicated.DedicatedServer {
        Server() {
            super(null, null, null, null, null, null, null, null, null, null);
        }
        @Override public RecipeManager getRecipeManager() { return recipes; }
    }

    static final class Connection extends ServerGamePacketListenerImpl {
        Connection() {
            super(null, null, null, null);
        }
        @Override public void send(net.minecraft.network.protocol.Packet<?> packet) { }
    }

    static final class World extends ServerLevel {
        Server server;
        World() {
            super(null, null, null, null, null, null, false, 0L, List.of(), false);
        }
        @Override public net.minecraft.server.MinecraftServer getServer() { return server; }
        @Override public RecipeManager recipeAccess() { return recipes; }
        @Override public boolean isClientSide() { return false; }
        @Override public net.minecraft.world.flag.FeatureFlagSet enabledFeatures() { return FeatureFlags.DEFAULT_FLAGS; }
        @Override public net.minecraft.core.RegistryAccess registryAccess() { return ParityData.access; }
        @Override public net.minecraft.world.level.gamerules.GameRules getGameRules() { return new net.minecraft.world.level.gamerules.GameRules(FeatureFlags.DEFAULT_FLAGS); }
        @Override public void levelEvent(net.minecraft.world.entity.Entity p, int type, BlockPos pos, int data) { }
        @Override public void playSeededSound(net.minecraft.world.entity.Entity p, double x, double y, double z, net.minecraft.core.Holder<net.minecraft.sounds.SoundEvent> s, net.minecraft.sounds.SoundSource c, float v, float pitch, long seed) { }
        @Override public boolean addFreshEntity(net.minecraft.world.entity.Entity e) { return true; }
        @Override public void gameEvent(net.minecraft.core.Holder<net.minecraft.world.level.gameevent.GameEvent> e, BlockPos p, net.minecraft.world.level.gameevent.GameEvent.Context c) { }
        // BlockEntity.setChanged of the "*_be" furnaces: no chunks here.
        @Override public void blockEntityChanged(BlockPos pos) { }
        @Override public void updateNeighbourForOutputSignal(BlockPos pos, net.minecraft.world.level.block.Block block) { }
    }

    static final class FakePlayer extends ServerPlayer {
        World world;
        boolean creative;
        Inventory inv;
        // No advancements are granted, so criterion triggers find no listeners.
        net.minecraft.server.PlayerAdvancements advancements;
        FakePlayer() {
            super(null, null, null, null);
        }
        @Override public Inventory getInventory() { return inv; }
        @Override public net.minecraft.server.PlayerAdvancements getAdvancements() { return advancements; }
        @Override public ServerLevel level() { return world; }
        @Override public boolean hasInfiniteMaterials() { return creative; }
        @Override public boolean isCreative() { return creative; }
        @Override public boolean isSpectator() { return false; }
        @Override public EntityType<?> getType() { return net.minecraft.world.entity.EntityTypes.PLAYER; }
        @Override public net.minecraft.core.Holder<EntityType<?>> typeHolder() { return net.minecraft.world.entity.EntityTypes.PLAYER.builtInRegistryHolder(); }
        @Override public boolean canUseSlot(EquipmentSlot slot) { return true; }
        @Override public void onEquipItem(EquipmentSlot slot, ItemStack old, ItemStack now) {
            if (!ItemStack.isSameItemSameComponents(old, now)) equips.add(slot.getName() + ":" + ParityData.canon(now));
        }
        @Override public boolean hasDisconnected() { return false; }
        @Override public ItemEntity drop(ItemStack stack, boolean thrown, net.minecraft.util.Prediction p) {
            if (!stack.isEmpty()) drops.add(ParityData.canon(stack));
            return null;
        }
        @Override public void triggerRecipeCrafted(net.minecraft.world.item.crafting.RecipeHolder<?> recipe, java.util.List<ItemStack> items) { }
        @Override public int awardRecipes(java.util.Collection<net.minecraft.world.item.crafting.RecipeHolder<?>> recipes) { return 0; }
        @Override public void awardStat(net.minecraft.stats.Stat<?> stat, int count) { }
        @Override public void giveExperiencePoints(int amount) { drops.add("xp+" + amount); }
        @Override public void giveExperienceLevels(int amount) { drops.add("levels" + (amount >= 0 ? "+" : "") + amount); }
    }

    static AbstractContainerMenu menu(String kind, FakePlayer player, Inventory inv) {
        var access = ContainerLevelAccess.create(player.world, BlockPos.ZERO);
        if (kind.startsWith("chest")) {
            int rows = Integer.parseInt(kind.substring(5));
            MenuType<?> type = new MenuType[]{MenuType.GENERIC_9x1, MenuType.GENERIC_9x2, MenuType.GENERIC_9x3, MenuType.GENERIC_9x4, MenuType.GENERIC_9x5, MenuType.GENERIC_9x6}[rows - 1];
            return new ChestMenu(type, 0, inv, new SimpleContainer(9 * rows), rows);
        }
        return switch (kind) {
            case "inventory" -> new InventoryMenu(inv, true, player);
            case "crafting" -> new CraftingMenu(0, inv, access);
            case "hopper" -> new HopperMenu(0, inv);
            case "dispenser" -> new DispenserMenu(0, inv);
            case "shulker" -> new ShulkerBoxMenu(0, inv);
            case "furnace" -> new FurnaceMenu(0, inv);
            case "blast_furnace" -> new BlastFurnaceMenu(0, inv);
            case "smoker" -> new SmokerMenu(0, inv);
            case "brewing" -> new BrewingStandMenu(0, inv);
            case "crafter" -> new CrafterMenu(0, inv);
            case "crafter_be" -> {
                // The server's menu: the block entity is the container and its data.
                crafter = new net.minecraft.world.level.block.entity.CrafterBlockEntity(BlockPos.ZERO,
                    net.minecraft.world.level.block.Blocks.CRAFTER.defaultBlockState());
                try {
                    Field data = net.minecraft.world.level.block.entity.CrafterBlockEntity.class.getDeclaredField("containerData");
                    data.setAccessible(true);
                    yield new CrafterMenu(0, inv, crafter, (ContainerData) data.get(crafter));
                } catch (ReflectiveOperationException e) {
                    throw new AssertionError(e);
                }
            }
            case "furnace_be", "blast_furnace_be", "smoker_be" -> {
                // The server's menu: the block entity is the container and its data,
                // so AbstractFurnaceBlockEntity.setItem restarts cooking.
                net.minecraft.world.level.block.entity.AbstractFurnaceBlockEntity be = switch (kind) {
                    case "furnace_be" -> new net.minecraft.world.level.block.entity.FurnaceBlockEntity(BlockPos.ZERO,
                        net.minecraft.world.level.block.Blocks.FURNACE.defaultBlockState());
                    case "blast_furnace_be" -> new net.minecraft.world.level.block.entity.BlastFurnaceBlockEntity(BlockPos.ZERO,
                        net.minecraft.world.level.block.Blocks.BLAST_FURNACE.defaultBlockState());
                    default -> new net.minecraft.world.level.block.entity.SmokerBlockEntity(BlockPos.ZERO,
                        net.minecraft.world.level.block.Blocks.SMOKER.defaultBlockState());
                };
                be.setLevel(player.world);
                try {
                    Field data = net.minecraft.world.level.block.entity.AbstractFurnaceBlockEntity.class.getDeclaredField("dataAccess");
                    data.setAccessible(true);
                    ContainerData furnaceData = (ContainerData) data.get(be);
                    yield switch (kind) {
                        case "furnace_be" -> new FurnaceMenu(0, inv, be, furnaceData);
                        case "blast_furnace_be" -> new BlastFurnaceMenu(0, inv, be, furnaceData);
                        default -> new SmokerMenu(0, inv, be, furnaceData);
                    };
                } catch (ReflectiveOperationException e) {
                    throw new AssertionError(e);
                }
            }
            case "loom" -> new LoomMenu(0, inv);
            default -> throw new IllegalArgumentException(kind);
        };
    }

    static String state(AbstractContainerMenu menu, Inventory inv) throws Exception {
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < menu.slots.size(); i++) {
            out.append(i > 0 ? " " : "").append(ParityData.canon(menu.getSlot(i).getItem()));
        }
        out.append(" | ").append(ParityData.canon(menu.getCarried()));
        Field dataSlots = AbstractContainerMenu.class.getDeclaredField("dataSlots");
        dataSlots.setAccessible(true);
        List<DataSlot> data = (List<DataSlot>) dataSlots.get(menu);
        out.append(" |");
        for (DataSlot d : data) out.append(' ').append(d.get());
        out.append(" | ").append(String.join(";", drops));
        return out.toString();
    }

    /** {@link #state} and what was equipped: the menu_click harness's line (place_recipe prints {@code state}). */
    static String clickState(AbstractContainerMenu menu, Inventory inv) throws Exception {
        return state(menu, inv) + " | " + String.join(";", equips);
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var field = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        field.setAccessible(true);
        unsafe = field.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        recipes = new RecipeManager(ParityData.lookup);
        recipes.finalizeRecipeLoading(FeatureFlags.VANILLA_SET);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                var f = JsonParser.parseString(line).getAsJsonObject();
                try {
                    drops.clear();
                    equips.clear();
                    var world = make(World.class);
                    world.server = make(Server.class);
                    var player = make(FakePlayer.class);
                    player.world = world;
                    player.creative = f.get("creative").getAsBoolean();
                    player.advancements = make(net.minecraft.server.PlayerAdvancements.class);
                    set(player.advancements, net.minecraft.server.PlayerAdvancements.class, "activeTriggers", new java.util.IdentityHashMap<>());
                    set(player, ServerPlayer.class, "connection", make(Connection.class));
                    var inv = new Inventory(player, new EntityEquipment());
                    player.inv = inv;
                    AbstractContainerMenu menu = menu(f.get("menu").getAsString(), player, inv);
                    set(player, Player.class, "containerMenu", menu);
                    if (menu instanceof InventoryMenu im) set(player, Player.class, "inventoryMenu", im);
                    var slotsJson = f.getAsJsonArray("slots");
                    for (int i = 0; i < menu.slots.size(); i++) menu.getSlot(i).set(ParityData.stack(slotsJson.get(i).getAsString()));
                    menu.setCarried(ParityData.stack(f.get("carried").getAsString()));
                    // The slot listener of the block crafter's menu refreshes the result.
                    if (f.get("menu").getAsString().equals("crafter_be")) menu.broadcastChanges();
                    var dataJson = f.getAsJsonArray("data");
                    for (int i = 0; i < dataJson.size(); i++) menu.setData(i, dataJson.get(i).getAsInt());
                    drops.clear();
                    equips.clear();
                    out.println("init " + clickState(menu, inv));
                    for (var opJson : f.getAsJsonArray("ops")) {
                        var op = opJson.getAsJsonArray();
                        int slot = op.get(0).getAsInt();
                        int button = op.get(1).getAsInt();
                        String kind = op.get(2).getAsString();
                        if (kind.equals("BUTTON")) {
                            // ServerGamePacketListenerImpl.handleContainerButtonClick
                            if (menu.clickMenuButton(player, slot)) menu.broadcastChanges();
                            out.println(clickState(menu, inv));
                            continue;
                        }
                        if (kind.equals("SLOT_STATE")) {
                            // handleContainerSlotStateChanged: CrafterBlockEntity.setSlotState
                            if (slot >= 0 && slot < 9) crafter.setSlotState(slot, button == 1);
                            out.println(clickState(menu, inv));
                            continue;
                        }
                        if (kind.equals("QUICK_MOVE_ALL")) {
                            // AbstractContainerScreen.mouseReleased on a Shift double click, lastQuickMoved
                            // being the op's fourth field.
                            var clicked = menu.getSlot(slot);
                            var moved = ParityData.stack(op.get(3).getAsString());
                            if (menu.canTakeItemForPickAll(ItemStack.EMPTY, clicked) && !moved.isEmpty()) {
                                for (var target : menu.slots) {
                                    if (target.mayPickup(player) && target.hasItem() && target.container == clicked.container
                                        && AbstractContainerMenu.canItemQuickReplace(target, moved, true)) {
                                        menu.clicked(target.index, 0, ContainerInput.QUICK_MOVE, player);
                                    }
                                }
                            }
                            out.println(clickState(menu, inv));
                            continue;
                        }
                        var input = ContainerInput.valueOf(kind);
                        try {
                            menu.clicked(slot, button, input, player);
                        } catch (RuntimeException e) {
                            Throwable cause = e.getCause() != null ? e.getCause() : e;
                            out.println("error " + cause.getClass().getSimpleName());
                            if (System.getenv("MENU_TRACE") != null) cause.printStackTrace();
                            continue;
                        }
                        out.println(clickState(menu, inv));
                    }
                    out.println("end");
                } catch (Throwable e) {
                    out.println("error-case " + e.getClass().getSimpleName());
                    if (System.getenv("MENU_TRACE") != null) e.printStackTrace();
                }
            }
        }
    }
}
