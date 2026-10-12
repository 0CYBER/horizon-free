package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodHandles;
import java.lang.invoke.MethodType;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Gui;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.GameRenderer;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySelector;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.level.Level;
import net.minecraft.world.scores.*;

/**
 * Vanilla's team rules in gameplay, run on stand-in entities whose teams and flags come from each case:
 * {@code Entity.doTeamsAllowDamage} (friendly fire), {@code Entity.isAlliedTo}, {@code EntitySelector.pushableBy}
 * (collision rules), {@code Entity.isInvisibleTo} (seeing invisible team mates) and
 * {@code LivingEntityRenderer.shouldShowName} (name tag visibility) with a stand-in client.
 *
 * <pre>  INPUTS OUTPUT</pre>
 */
public final class VanillaTeamRulesOracle {
    static Object unsafe;
    static Method allocate;

    @SuppressWarnings("unchecked")
    static <T> T alloc(Class<T> type) throws Exception {
        return (T) allocate.invoke(unsafe, type);
    }

    static void put(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    static final class Stub extends Zombie {
        PlayerTeam team;
        boolean pushable, spectator, invisible, discrete, vehicle;
        Level home;

        Stub() {
            super((Level) null);
        }

        @Override
        public PlayerTeam getTeam() {
            return this.team;
        }

        @Override
        public boolean isPushable() {
            return this.pushable;
        }

        @Override
        public boolean isSpectator() {
            return this.spectator;
        }

        @Override
        public boolean isInvisible() {
            return this.invisible;
        }

        @Override
        public boolean isDiscrete() {
            return this.discrete;
        }

        @Override
        public boolean isVehicle() {
            return this.vehicle;
        }

        @Override
        public Level level() {
            return this.home;
        }
    }

    static final class Viewer extends LocalPlayer {
        PlayerTeam team;
        boolean spectator;

        Viewer() {
            super(null, null, null, null, null, null, false, null, null);
        }

        @Override
        public PlayerTeam getTeam() {
            return this.team;
        }

        @Override
        public boolean isSpectator() {
            return this.spectator;
        }
    }

    abstract static class Renderer extends LivingEntityRenderer<LivingEntity, LivingEntityRenderState, net.minecraft.client.model.EntityModel<LivingEntityRenderState>> {
        Renderer() {
            super(null, null, 0.0F);
        }
    }

    static final class Concrete extends Renderer {
        @Override
        public Identifier getTextureLocation(final LivingEntityRenderState state) {
            return null;
        }

        @Override
        public LivingEntityRenderState createRenderState() {
            return null;
        }
    }

    static void options(PlayerTeam t, JsonObject o) {
        t.setAllowFriendlyFire(o.get("ff").getAsBoolean());
        t.setSeeFriendlyInvisibles(o.get("si").getAsBoolean());
        for (Team.CollisionRule r : Team.CollisionRule.values()) {
            if (r.getSerializedName().equals(o.get("collision").getAsString())) t.setCollisionRule(r);
        }
        for (Team.Visibility v : Team.Visibility.values()) {
            if (v.getSerializedName().equals(o.get("nametag").getAsString())) t.setNameTagVisibility(v);
        }
    }

    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        unsafe = unsafeField.get(null);
        allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        MethodHandle shouldShowName = MethodHandles.privateLookupIn(LivingEntityRenderer.class, MethodHandles.lookup())
            .findVirtual(LivingEntityRenderer.class, "shouldShowName", MethodType.methodType(boolean.class, LivingEntity.class, double.class));
        Renderer renderer = alloc(Concrete.class);
        ServerLevel level = alloc(ServerLevel.class);
        Gson gson = new GsonBuilder().serializeNulls().create();
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             BufferedReader in = Files.newBufferedReader(Path.of(args[0]), StandardCharsets.UTF_8)) {
            String line;
            while ((line = in.readLine()) != null) {
                JsonObject c = JsonParser.parseString(line).getAsJsonObject();
                Scoreboard board = new Scoreboard();
                PlayerTeam a = board.addPlayerTeam("A");
                PlayerTeam b = board.addPlayerTeam("B");
                options(a, c.getAsJsonObject("A"));
                options(b, c.getAsJsonObject("B"));
                java.util.function.Function<String, PlayerTeam> team = k -> c.get(k).isJsonNull() ? null : c.get(k).getAsString().equals("A") ? a : b;
                Stub own = alloc(Stub.class);
                own.team = team.apply("own");
                own.home = level;
                Stub other = alloc(Stub.class);
                other.team = team.apply("other");
                other.home = level;
                other.pushable = c.get("pushable").getAsBoolean();
                other.spectator = c.get("spectator").getAsBoolean();
                other.invisible = c.get("invisible").getAsBoolean();
                other.discrete = c.get("discrete").getAsBoolean();
                other.vehicle = c.get("vehicle").getAsBoolean();
                Viewer viewer = alloc(Viewer.class);
                viewer.team = team.apply("viewer");
                viewer.spectator = c.get("viewer_spectator").getAsBoolean();
                Minecraft mc = alloc(Minecraft.class);
                put(mc, Minecraft.class, "player", viewer);
                Camera camera = alloc(Camera.class);
                put(camera, Camera.class, "entity", c.get("camera_is_entity").getAsBoolean() ? other : viewer);
                GameRenderer gameRenderer = alloc(GameRenderer.class);
                put(gameRenderer, GameRenderer.class, "mainCamera", camera);
                put(mc, Minecraft.class, "gameRenderer", gameRenderer);
                Hud hud = alloc(Hud.class);
                put(hud, Hud.class, "isHidden", c.get("hud_hidden").getAsBoolean());
                Gui gui = alloc(Gui.class);
                put(gui, Gui.class, "hud", hud);
                put(mc, Minecraft.class, "gui", gui);
                put(null, Minecraft.class, "instance", mc);
                JsonObject r = new JsonObject();
                r.add("case", c.get("case"));
                try {
                    r.addProperty("damage", own.doTeamsAllowDamage(other));
                    r.addProperty("allied", own.isAlliedTo((Entity) other));
                    r.addProperty("pushes", EntitySelector.pushableBy(own).test(other));
                    r.addProperty("invisible", other.isInvisibleTo(viewer));
                    r.addProperty("name", (boolean) shouldShowName.invoke(renderer, (LivingEntity) other, c.get("distance").getAsDouble()));
                } catch (Throwable e) {
                    r.addProperty("error", e.toString());
                }
                out.println(gson.toJson(r));
            }
        }
    }
}
