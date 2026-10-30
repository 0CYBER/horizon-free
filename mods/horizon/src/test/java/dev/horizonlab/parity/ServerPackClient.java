package dev.horizonlab.parity;

import com.mojang.serialization.JsonOps;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.BufferedReader;
import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.zip.Deflater;
import java.util.zip.Inflater;
import net.minecraft.SharedConstants;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.common.ClientboundDisconnectPacket;
import net.minecraft.network.protocol.common.ClientboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ClientboundPingPacket;
import net.minecraft.network.protocol.common.ClientboundResourcePackPushPacket;
import net.minecraft.network.protocol.common.ServerboundClientInformationPacket;
import net.minecraft.network.protocol.common.ServerboundCustomPayloadPacket;
import net.minecraft.network.protocol.common.ServerboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ServerboundPongPacket;
import net.minecraft.network.protocol.common.ServerboundResourcePackPacket;
import net.minecraft.network.protocol.common.custom.BrandPayload;
import net.minecraft.network.protocol.configuration.ClientboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ClientboundSelectKnownPacks;
import net.minecraft.network.protocol.configuration.ConfigurationProtocols;
import net.minecraft.network.protocol.configuration.ServerboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ServerboundSelectKnownPacks;
import net.minecraft.network.protocol.game.ClientboundLoginPacket;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.handshake.ClientIntent;
import net.minecraft.network.protocol.handshake.ClientIntentionPacket;
import net.minecraft.network.protocol.handshake.HandshakeProtocols;
import net.minecraft.network.protocol.login.ClientboundLoginCompressionPacket;
import net.minecraft.network.protocol.login.ClientboundLoginDisconnectPacket;
import net.minecraft.network.protocol.login.ClientboundLoginFinishedPacket;
import net.minecraft.network.protocol.login.LoginProtocols;
import net.minecraft.network.protocol.login.ServerboundHelloPacket;
import net.minecraft.network.protocol.login.ServerboundLoginAcknowledgedPacket;
import net.minecraft.server.level.ClientInformation;

/**
 * Players made of vanilla's own protocol code that answer a server's resource pack offer
 * (tools/test_server_resource_pack_parity.py). Script on stdin: {@code join NAME DELAY ACTIONS} joins and, once the
 * pack is offered, waits DELAY seconds (answering keep-alives) then sends ACTIONS, a comma-separated list of
 * ServerboundResourcePackPacket.Action names (or {@code none}); it returns once the player is in play or
 * disconnected. {@code play NAME ACTION} sends an answer to the same pack in play and waits up to 10 s for a disconnect;
 * {@code end} writes the transcript. Each player records the configuration packets it was sent (runs of one packet
 * counted), the offer, whether it was kept alive while it waited, and how the connection ended.
 */
public final class ServerPackClient {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        String host = args[0];
        int port = Integer.parseInt(args[1]);
        Path output = Path.of(args[2]);
        Map<String, Player> players = new LinkedHashMap<>();
        BufferedReader script = new BufferedReader(new InputStreamReader(System.in, StandardCharsets.UTF_8));
        String line;
        while ((line = script.readLine()) != null) {
            String[] w = line.trim().split(" ");
            try {
                switch (w[0]) {
                    case "join" -> {
                        Player p = new Player(w[1]);
                        players.put(w[1], p);
                        p.join(host, port, Double.parseDouble(w[2]), w[3]);
                    }
                    case "play" -> players.get(w[1]).answerInPlay(action(w[2]));
                    case "end" -> {
                        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(output, StandardCharsets.UTF_8))) {
                            for (Player p : players.values()) {
                                synchronized (p.lines) {
                                    for (String l : p.lines) out.println(p.name + " " + l);
                                }
                            }
                        }
                        for (Player p : players.values()) p.close();
                        System.exit(0);
                    }
                    default -> throw new IllegalArgumentException("unknown script line " + line);
                }
            } catch (Exception e) {
                Player p = w.length > 1 ? players.get(w[1]) : null;
                if (p != null) p.record("error " + w[0] + " " + e);
                else System.err.println("error " + line + ": " + e);
            }
        }
        System.exit(1);
    }

    private static ServerboundResourcePackPacket.Action action(String name) {
        return ServerboundResourcePackPacket.Action.valueOf(name.toUpperCase(Locale.ROOT));
    }

    private static String json(Component c) {
        return ComponentSerialization.CODEC.encodeStart(ParityData.access.createSerializationContext(JsonOps.INSTANCE), c)
            .getOrThrow().toString();
    }

    private static final class Player {
        final String name;
        final List<String> lines = new ArrayList<>();
        private Socket socket;
        private DataInputStream in;
        private OutputStream out;
        private int threshold = -1;
        private ProtocolInfo<?> inbound;
        private ProtocolInfo<?> outbound;
        private final Object ended = new Object();
        private boolean disconnected;
        /** The pack offered during configuration (the one answered in play). */
        private java.util.UUID offered;
        private String lastConfig;
        private int run;

        Player(String name) {
            this.name = name;
        }

        void record(String line) {
            synchronized (lines) {
                lines.add(line);
            }
        }

        /** A configuration packet: consecutive ones of the same type are one line with their count. */
        private void config(Packet<?> p) {
            String type = p.type().id().getPath();
            if (type.equals(lastConfig)) {
                run++;
                return;
            }
            flushConfig();
            lastConfig = type;
            run = 1;
        }

        private void flushConfig() {
            if (lastConfig != null) record("config " + lastConfig + (run > 1 ? " x" + run : ""));
            lastConfig = null;
        }

        void close() throws Exception {
            if (socket != null) socket.close();
        }

        void answerInPlay(ServerboundResourcePackPacket.Action action) throws Exception {
            synchronized (ended) {
                send(new ServerboundResourcePackPacket(offered, action));
                long until = System.nanoTime() + 10_000_000_000L;
                while (!disconnected) {
                    long left = (until - System.nanoTime()) / 1_000_000L;
                    if (left <= 0) {
                        record("still connected after " + action);
                        return;
                    }
                    ended.wait(left);
                }
            }
        }

        void join(String host, int port, double delay, String actions) throws Exception {
            socket = new Socket();
            socket.connect(new InetSocketAddress(host, port), 10_000);
            socket.setTcpNoDelay(true);
            in = new DataInputStream(socket.getInputStream());
            out = socket.getOutputStream();
            outbound = HandshakeProtocols.SERVERBOUND;
            send(new ClientIntentionPacket(SharedConstants.getProtocolVersion(), host, port, ClientIntent.LOGIN));
            outbound = LoginProtocols.SERVERBOUND;
            inbound = LoginProtocols.CLIENTBOUND;
            send(new ServerboundHelloPacket(name, UUIDUtil.createOfflinePlayerUUID(name)));
            while (true) {
                Packet<?> p = read();
                if (p instanceof ClientboundLoginCompressionPacket c) {
                    threshold = c.getCompressionThreshold();
                } else if (p instanceof ClientboundLoginFinishedPacket) {
                    send(ServerboundLoginAcknowledgedPacket.INSTANCE);
                    break;
                } else if (p instanceof ClientboundLoginDisconnectPacket d) {
                    record("login refused " + d.reason().getString());
                    return;
                }
            }
            outbound = ConfigurationProtocols.SERVERBOUND;
            inbound = ConfigurationProtocols.CLIENTBOUND;
            send(new ServerboundCustomPayloadPacket(new BrandPayload("vanilla")));
            send(new ServerboundClientInformationPacket(ClientInformation.createDefault()));
            boolean keptAlive = false;
            while (true) {
                Packet<?> p;
                try {
                    p = read();
                } catch (java.io.EOFException | java.net.SocketException e) {
                    flushConfig();
                    record("connection closed in configuration");
                    return;
                }
                if (!(p instanceof ClientboundKeepAlivePacket)) config(p);
                if (p instanceof ClientboundSelectKnownPacks k) {
                    send(new ServerboundSelectKnownPacks(k.knownPacks()));
                } else if (p instanceof ClientboundKeepAlivePacket k) {
                    // Kept alive while the player looks at the prompt (once is enough to know).
                    if (offered != null && !keptAlive) {
                        flushConfig();
                        record("kept alive while the offer was open");
                        keptAlive = true;
                    }
                    send(new ServerboundKeepAlivePacket(k.getId()));
                } else if (p instanceof ClientboundPingPacket k) {
                    send(new ServerboundPongPacket(k.getId()));
                } else if (p instanceof ClientboundResourcePackPushPacket push) {
                    flushConfig();
                    record("offer id=" + push.id() + " url=" + push.url() + " hash=" + push.hash() + " required="
                        + push.required() + " prompt=" + push.prompt().map(ServerPackClient::json).orElse("none"));
                    offered = push.id();
                    Thread answer = new Thread(() -> {
                        try {
                            Thread.sleep((long) (delay * 1000));
                            if (actions.equals("none")) return;
                            for (String a : actions.split(",")) send(new ServerboundResourcePackPacket(push.id(), action(a)));
                        } catch (Exception e) {
                            record("error answering " + e);
                        }
                    }, name + "-answer");
                    answer.setDaemon(true);
                    answer.start();
                } else if (p instanceof ClientboundFinishConfigurationPacket) {
                    flushConfig();
                    send(ServerboundFinishConfigurationPacket.INSTANCE);
                    break;
                } else if (p instanceof ClientboundDisconnectPacket d) {
                    flushConfig();
                    record("disconnected in configuration: " + d.reason().getString());
                    return;
                }
            }
            outbound = GameProtocols.SERVERBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access),
                new GameProtocols.Context() {
                    @Override
                    public boolean hasInfiniteMaterials() {
                        return false;
                    }

                    @Override
                    public boolean canUseCommandBlocks() {
                        return false;
                    }
                });
            inbound = GameProtocols.CLIENTBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access));
            while (!(read() instanceof ClientboundLoginPacket)) {
            }
            record("joined");
            Thread reader = new Thread(this::play, name);
            reader.setDaemon(true);
            reader.start();
        }

        private void play() {
            try {
                while (true) {
                    Packet<?> p = read();
                    if (p instanceof ClientboundKeepAlivePacket k) {
                        send(new ServerboundKeepAlivePacket(k.getId()));
                    } else if (p instanceof ClientboundPingPacket k) {
                        send(new ServerboundPongPacket(k.getId()));
                    } else if (p instanceof ClientboundDisconnectPacket d) {
                        record("disconnected in play: " + d.reason().getString());
                        synchronized (ended) {
                            disconnected = true;
                            ended.notifyAll();
                        }
                        return;
                    }
                }
            } catch (Exception e) {
                if (!socket.isClosed()) record("error play " + e);
            }
        }

        private Packet<?> read() throws Exception {
            byte[] payload = readFrame();
            ByteBuf buf = Unpooled.wrappedBuffer(payload);
            Packet<?> packet = inbound.codec().decode(buf);
            if (buf.readableBytes() > 0) {
                throw new IllegalStateException(packet.type().id() + " was larger than expected: " + buf.readableBytes() + " bytes left");
            }
            return packet;
        }

        synchronized void send(Packet<?> packet) throws Exception {
            ByteBuf body = Unpooled.buffer();
            ((ProtocolInfo) outbound).codec().encode(body, packet);
            byte[] raw = new byte[body.readableBytes()];
            body.readBytes(raw);
            ByteBuf frame = Unpooled.buffer();
            if (threshold >= 0) {
                ByteBuf inner = Unpooled.buffer();
                if (raw.length >= threshold) {
                    writeVarInt(inner, raw.length);
                    Deflater d = new Deflater();
                    d.setInput(raw);
                    d.finish();
                    byte[] tmp = new byte[raw.length + 64];
                    while (!d.finished()) inner.writeBytes(tmp, 0, d.deflate(tmp));
                    d.end();
                } else {
                    writeVarInt(inner, 0);
                    inner.writeBytes(raw);
                }
                writeVarInt(frame, inner.readableBytes());
                frame.writeBytes(inner);
            } else {
                writeVarInt(frame, raw.length);
                frame.writeBytes(raw);
            }
            byte[] bytes = new byte[frame.readableBytes()];
            frame.readBytes(bytes);
            out.write(bytes);
            out.flush();
        }

        private byte[] readFrame() throws Exception {
            byte[] frame = new byte[readVarInt(in)];
            in.readFully(frame);
            if (threshold < 0) return frame;
            ByteArrayInputStream header = new ByteArrayInputStream(frame);
            int dataLength = readVarInt(header);
            int skip = frame.length - header.available();
            if (dataLength == 0) return java.util.Arrays.copyOfRange(frame, skip, frame.length);
            Inflater inflater = new Inflater();
            inflater.setInput(frame, skip, frame.length - skip);
            byte[] raw = new byte[dataLength];
            int n = inflater.inflate(raw);
            inflater.end();
            if (n != dataLength) throw new IllegalStateException("decompressed " + n + " of " + dataLength + " bytes");
            return raw;
        }
    }

    private static int readVarInt(InputStream in) throws Exception {
        int value = 0;
        for (int shift = 0; shift < 35; shift += 7) {
            int b = in.read();
            if (b < 0) throw new java.io.EOFException();
            value |= (b & 0x7F) << shift;
            if ((b & 0x80) == 0) return value;
        }
        throw new IllegalStateException("VarInt too big");
    }

    private static void writeVarInt(ByteBuf buf, int value) {
        while ((value & ~0x7F) != 0) {
            buf.writeByte((value & 0x7F) | 0x80);
            value >>>= 7;
        }
        buf.writeByte(value);
    }
}
