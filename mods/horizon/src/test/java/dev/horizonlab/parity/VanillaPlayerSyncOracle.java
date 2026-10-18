package dev.horizonlab.parity;

import com.google.gson.*;
import io.netty.buffer.Unpooled;
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.HexFormat;
import net.minecraft.client.*;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.world.entity.*;
import net.minecraft.world.phys.Vec3;

/** Runs shipped methods; only the world lifecycle and packet sink are fixtures. */
public final class VanillaPlayerSyncOracle {
    static Object unsafe;
    static Method allocate;
    static PrintWriter out;
    static Field field(Class<?> type, String name) throws Exception {
        var f = type.getDeclaredField(name); f.setAccessible(true); return f;
    }
    static <T> T make(Class<T> type) throws Exception {
        return type.cast(allocate.invoke(unsafe, type));
    }
    public static final class Player extends LocalPlayer {
        boolean ground;
        public Player() { super(null, null, null, null, null, null, false, null, null); }
        @Override protected boolean isControlledCamera() { return true; }
        @Override public boolean isSprinting() { return false; }
        @Override public boolean onGround() { return ground; }
    }
    public static final class Sink extends ClientPacketListener {
        boolean sent;
        public Sink() { super(null, null, null); }
        @Override public void send(Packet<?> packet) {
            sent = true;
            var buf = new FriendlyByteBuf(Unpooled.buffer());
            String name;
            try {
                if (packet instanceof ServerboundMovePlayerPacket.PosRot p) {
                    name = "posrot"; ServerboundMovePlayerPacket.PosRot.STREAM_CODEC.encode(buf, p);
                } else if (packet instanceof ServerboundMovePlayerPacket.Pos p) {
                    name = "pos"; ServerboundMovePlayerPacket.Pos.STREAM_CODEC.encode(buf, p);
                } else if (packet instanceof ServerboundMovePlayerPacket.Rot p) {
                    name = "rot"; ServerboundMovePlayerPacket.Rot.STREAM_CODEC.encode(buf, p);
                } else if (packet instanceof ServerboundMovePlayerPacket.StatusOnly p) {
                    name = "status"; ServerboundMovePlayerPacket.StatusOnly.STREAM_CODEC.encode(buf, p);
                } else { throw new AssertionError(packet); }
                byte[] bytes = new byte[buf.readableBytes()]; buf.readBytes(bytes);
                out.println(name + " " + HexFormat.of().formatHex(bytes));
            } finally { buf.release(); }
        }
    }
    static double number(JsonElement value) {
        return Double.longBitsToDouble(Long.parseUnsignedLong(value.getAsString(), 16));
    }
    static Vec3 vector(JsonObject row, String key) {
        var a = row.getAsJsonArray(key);
        return new Vec3(number(a.get(0)), number(a.get(1)), number(a.get(2)));
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var u = Class.forName("sun.misc.Unsafe");
        unsafe = field(u, "theUnsafe").get(null);
        allocate = u.getMethod("allocateInstance", Class.class);
        var sendPosition = LocalPlayer.class.getDeclaredMethod("sendPosition");
        sendPosition.setAccessible(true);
        var minecraft = make(Minecraft.class);
        var options = make(Options.class);
        field(Options.class, "autoJump").set(options, OptionInstance.createBoolean("test", false));
        field(Minecraft.class, "options").set(minecraft, options);
        try (var output = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])));
             var lines = Files.lines(Path.of(args[0]))) {
            out = output;
            for (String line : (Iterable<String>) lines::iterator) {
                var row = JsonParser.parseString(line).getAsJsonObject();
                if (row.get("kind").getAsString().equals("movement")) {
                    var p = make(Player.class);
                    var sink = make(Sink.class);
                    field(LocalPlayer.class, "connection").set(p, sink);
                    field(LocalPlayer.class, "minecraft").set(p, minecraft);
                    for (var value : row.getAsJsonArray("ops")) {
                        var op = value.getAsJsonArray();
                        field(Entity.class, "position").set(p, new Vec3(number(op.get(0)), number(op.get(1)), number(op.get(2))));
                        field(Entity.class, "yRot").setFloat(p, (float)number(op.get(3)));
                        field(Entity.class, "xRot").setFloat(p, (float)number(op.get(4)));
                        p.ground = op.get(5).getAsBoolean();
                        p.horizontalCollision = op.get(6).getAsBoolean();
                        sink.sent = false;
                        sendPosition.invoke(p);
                        if (!sink.sent) out.println("-");
                    }
                } else {
                    var old = row.getAsJsonArray("old");
                    var next = row.getAsJsonArray("new");
                    var base = new PositionMoveRotation(Vec3.ZERO, vector(row, "velocity"), (float)number(old.get(0)), (float)number(old.get(1)));
                    var change = new PositionMoveRotation(Vec3.ZERO, vector(row, "delta"), (float)number(next.get(0)), (float)number(next.get(1)));
                    var v = PositionMoveRotation.calculateAbsolute(base, change, Relative.unpack(row.get("flags").getAsInt())).deltaMovement();
                    out.printf("%016x %016x %016x%n", Double.doubleToRawLongBits(v.x), Double.doubleToRawLongBits(v.y), Double.doubleToRawLongBits(v.z));
                }
            }
        }
    }
}
