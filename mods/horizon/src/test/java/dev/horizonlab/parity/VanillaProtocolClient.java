package dev.horizonlab.parity;

import com.google.gson.JsonElement;
import com.mojang.serialization.JsonOps;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.DataInputStream;
import java.io.EOFException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.ArrayList;
import java.util.BitSet;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.TreeMap;
import java.util.UUID;
import java.util.zip.DataFormatException;
import java.util.zip.Deflater;
import java.util.zip.Inflater;
import net.minecraft.SharedConstants;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.chat.LastSeenMessages;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.protocol.common.ClientboundCustomPayloadPacket;
import net.minecraft.network.protocol.common.ClientboundDisconnectPacket;
import net.minecraft.network.protocol.common.ClientboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ClientboundPingPacket;
import net.minecraft.network.protocol.common.ServerboundClientInformationPacket;
import net.minecraft.network.protocol.common.ServerboundCustomPayloadPacket;
import net.minecraft.network.protocol.common.ServerboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ServerboundPongPacket;
import net.minecraft.network.protocol.common.custom.BrandPayload;
import net.minecraft.network.protocol.configuration.ClientboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ClientboundSelectKnownPacks;
import net.minecraft.network.protocol.configuration.ConfigurationProtocols;
import net.minecraft.network.protocol.configuration.ServerboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ServerboundSelectKnownPacks;
import net.minecraft.network.protocol.game.ClientboundChunkBatchFinishedPacket;
import net.minecraft.network.protocol.game.ClientboundDisguisedChatPacket;
import net.minecraft.network.protocol.game.ClientboundLevelChunkWithLightPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerChatPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket;
import net.minecraft.network.protocol.game.ClientboundSystemChatPacket;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.game.ServerboundAcceptTeleportationPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.network.protocol.game.ServerboundChatPacket;
import net.minecraft.network.protocol.game.ServerboundChunkBatchReceivedPacket;
import net.minecraft.network.protocol.game.ServerboundMovePlayerPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerLoadedPacket;
import net.minecraft.network.protocol.handshake.ClientIntent;
import net.minecraft.network.protocol.handshake.ClientIntentionPacket;
import net.minecraft.network.protocol.handshake.HandshakeProtocols;
import net.minecraft.network.protocol.login.ClientboundLoginCompressionPacket;
import net.minecraft.network.protocol.login.ClientboundLoginDisconnectPacket;
import net.minecraft.network.protocol.login.ClientboundLoginFinishedPacket;
import net.minecraft.network.protocol.login.LoginProtocols;
import net.minecraft.network.protocol.login.ServerboundHelloPacket;
import net.minecraft.network.protocol.login.ServerboundLoginAcknowledgedPacket;
import net.minecraft.network.protocol.ping.ClientboundPongResponsePacket;
import net.minecraft.network.protocol.ping.ServerboundPingRequestPacket;
import net.minecraft.network.protocol.status.ClientboundStatusResponsePacket;
import net.minecraft.network.protocol.status.ServerStatus;
import net.minecraft.network.protocol.status.ServerboundStatusRequestPacket;
import net.minecraft.network.protocol.status.StatusProtocols;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.world.entity.PositionMoveRotation;
import net.minecraft.world.phys.Vec3;

/**
 * A headless client made of vanilla's own protocol: it frames, compresses and
 * decodes every packet with the codecs of the pinned server JAR (the client
 * shares them), so any byte a server sends that vanilla would not read is
 * recorded as an error. tools/test_dedicated_server_parity.py points it at a
 * vanilla dedicated server and at Horizon's, with the same settings and
 * world, and compares the transcripts.
 *
 * <p>{@code VanillaProtocolClient HOST PORT NAME OUT} pings the server (status
 * and ping), joins as NAME in offline mode, goes through login and
 * configuration, accepts the spawn teleport, waits for the chunks around the
 * spawn, sends a chat message and the {@code /list} command, and leaves.
 * Each transcript line is a section and a summary: the login and
 * configuration packets in order; in play, the packets up to the first chunk
 * batch in order, then the chunks by position with the hash of their bytes
 * and the replies to the chat and command. Values that change between runs
 * (entity ids, keep-alive ids, times, salts) are not part of the summaries.
 */
public final class VanillaProtocolClient {
    private Socket socket;
    private DataInputStream in;
    private OutputStream out;
    private int threshold = -1;
    private final List<String> lines = new ArrayList<>();
    private ProtocolInfo<?> inbound;
    private ProtocolInfo<?> outbound;
    private String state;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        String host = args[0];
        int port = Integer.parseInt(args[1]);
        String name = args[2];
        Path output = Path.of(args[3]);
        VanillaProtocolClient client = new VanillaProtocolClient();
        try {
            client.status(host, port);
            client.join(host, port, name);
        } catch (Exception e) {
            client.lines.add("error " + client.state + " " + e);
        } finally {
            if (client.socket != null) client.socket.close();
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(output, StandardCharsets.UTF_8))) {
            for (String line : client.lines) w.println(line);
        }
        System.exit(0);
    }

    // ---- connection ---------------------------------------------------

    private void open(String host, int port) throws Exception {
        socket = new Socket();
        socket.connect(new InetSocketAddress(host, port), 10_000);
        socket.setSoTimeout(30_000);
        socket.setTcpNoDelay(true);
        in = new DataInputStream(socket.getInputStream());
        out = socket.getOutputStream();
        threshold = -1;
    }

    private static int readVarInt(InputStream in) throws Exception {
        int value = 0;
        for (int i = 0; i < 5; i++) {
            int b = in.read();
            if (b < 0) throw new EOFException();
            value |= (b & 0x7F) << (7 * i);
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

    @SuppressWarnings({"unchecked", "rawtypes"})
    private void send(Packet<?> packet) throws Exception {
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
                while (!d.finished()) {
                    int n = d.deflate(tmp);
                    inner.writeBytes(tmp, 0, n);
                }
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

    /** The next packet's raw payload, decompressed as CompressionDecoder does (validating). */
    private byte[] readFrame() throws Exception {
        int length = readVarInt(in);
        byte[] frame = new byte[length];
        in.readFully(frame);
        if (threshold < 0) return frame;
        ByteBuf buf = Unpooled.wrappedBuffer(frame);
        int dataLength = readVarInt(new java.io.ByteArrayInputStream(frame));
        int header = varIntSize(dataLength);
        buf.skipBytes(header);
        if (dataLength == 0) {
            byte[] raw = new byte[buf.readableBytes()];
            buf.readBytes(raw);
            return raw;
        }
        if (dataLength < threshold) {
            throw new IllegalStateException("Badly compressed packet - size of " + dataLength + " is below server threshold of " + threshold);
        }
        if (dataLength > 8388608) {
            throw new IllegalStateException("Badly compressed packet - size of " + dataLength + " is larger than protocol maximum of 8388608");
        }
        Inflater inflater = new Inflater();
        inflater.setInput(frame, header, frame.length - header);
        byte[] raw = new byte[dataLength];
        int n;
        try {
            n = inflater.inflate(raw);
        } catch (DataFormatException e) {
            throw new IllegalStateException("bad zlib data", e);
        }
        boolean finished = inflater.finished();
        inflater.end();
        if (n != dataLength || !finished) throw new IllegalStateException("decompressed " + n + " of " + dataLength + " bytes");
        return raw;
    }

    private static int varIntSize(int v) {
        int n = 1;
        while ((v & ~0x7F) != 0) {
            v >>>= 7;
            n++;
        }
        return n;
    }

    private record Received(Packet<?> packet, byte[] payload, String type) {}

    /** The next packet, decoded with the inbound protocol; leftover bytes are an error (PacketDecoder). */
    private Received read() throws Exception {
        byte[] payload = readFrame();
        ByteBuf buf = Unpooled.wrappedBuffer(payload);
        Packet<?> packet = inbound.codec().decode(buf);
        if (buf.readableBytes() > 0) {
            throw new IllegalStateException("Packet " + inbound.id().id() + "/" + packet.type().id() + " was larger than I expected, found "
                + buf.readableBytes() + " bytes extra whilst reading");
        }
        return new Received(packet, payload, packet.type().id().toString());
    }

    // ---- summaries ----------------------------------------------------

    private static String sha(byte[] bytes) {
        try {
            return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes)).substring(0, 16);
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    private static String json(Component c) {
        JsonElement e = ComponentSerialization.CODEC.encodeStart(ParityData.access.createSerializationContext(JsonOps.INSTANCE), c).getOrThrow();
        return e.toString();
    }

    /** A record's text without object identities. */
    private static String text(Object o) {
        return String.valueOf(o).replaceAll("@[0-9a-f]{4,}", "@");
    }

    private void record(String section, String line) {
        lines.add(section + " " + line);
    }

    // ---- status -------------------------------------------------------

    private void status(String host, int port) throws Exception {
        state = "status";
        open(host, port);
        outbound = HandshakeProtocols.SERVERBOUND;
        send(new ClientIntentionPacket(SharedConstants.getProtocolVersion(), host, port, ClientIntent.STATUS));
        outbound = StatusProtocols.SERVERBOUND;
        inbound = StatusProtocols.CLIENTBOUND;
        send(ServerboundStatusRequestPacket.INSTANCE);
        Received r = read();
        if (r.packet instanceof ClientboundStatusResponsePacket p) {
            JsonElement e = ServerStatus.CODEC.encodeStart(JsonOps.INSTANCE, p.status()).getOrThrow();
            // The favicon is compared by its bytes' hash.
            String s = e.toString().replaceAll("\"favicon\":\"[^\"]*\"", "\"favicon\":\"" + p.status().favicon().map(f -> sha(f.iconBytes())).orElse("") + "\"");
            record("status", r.type + " " + s);
        } else {
            record("status", r.type + " " + text(r.packet));
        }
        send(new ServerboundPingRequestPacket(1234567890123L));
        Received pong = read();
        record("status", pong.type + " " + (pong.packet instanceof ClientboundPongResponsePacket p ? p.time() : text(pong.packet)));
        int after = in.read();
        record("status", "closed-after-pong " + (after < 0));
        socket.close();
    }

    // ---- join ---------------------------------------------------------

    private void join(String host, int port, String name) throws Exception {
        open(host, port);
        state = "login";
        outbound = HandshakeProtocols.SERVERBOUND;
        send(new ClientIntentionPacket(SharedConstants.getProtocolVersion(), host, port, ClientIntent.LOGIN));
        outbound = LoginProtocols.SERVERBOUND;
        inbound = LoginProtocols.CLIENTBOUND;
        UUID uuid = UUIDUtil.createOfflinePlayerUUID(name);
        send(new ServerboundHelloPacket(name, uuid));
        while (true) {
            Received r = read();
            if (r.packet instanceof ClientboundLoginCompressionPacket p) {
                threshold = p.getCompressionThreshold();
                record("login", r.type + " " + threshold);
            } else if (r.packet instanceof ClientboundLoginFinishedPacket p) {
                record("login", r.type + " " + p.gameProfile().id() + " " + p.gameProfile().name() + " " + p.gameProfile().properties().size());
                send(ServerboundLoginAcknowledgedPacket.INSTANCE);
                break;
            } else if (r.packet instanceof ClientboundLoginDisconnectPacket p) {
                record("login", r.type + " " + json(p.reason()));
                return;
            } else {
                record("login", r.type + " " + text(r.packet));
            }
        }
        state = "configuration";
        outbound = ConfigurationProtocols.SERVERBOUND;
        inbound = ConfigurationProtocols.CLIENTBOUND;
        // ClientHandshakePacketListenerImpl.handleLoginFinished: brand, then the options.
        send(new ServerboundCustomPayloadPacket(new BrandPayload("vanilla")));
        send(new ServerboundClientInformationPacket(ClientInformation.createDefault()));
        while (true) {
            Received r = read();
            if (r.packet instanceof ClientboundSelectKnownPacks p) {
                record("config", r.type + " " + text(p.knownPacks()));
                send(new ServerboundSelectKnownPacks(p.knownPacks()));
            } else if (r.packet instanceof ClientboundKeepAlivePacket p) {
                record("config", r.type);
                send(new ServerboundKeepAlivePacket(p.getId()));
            } else if (r.packet instanceof ClientboundPingPacket p) {
                record("config", r.type);
                send(new ServerboundPongPacket(p.getId()));
            } else if (r.packet instanceof ClientboundCustomPayloadPacket p) {
                record("config", r.type + " " + text(p.payload()));
            } else if (r.packet instanceof ClientboundFinishConfigurationPacket) {
                record("config", r.type);
                send(ServerboundFinishConfigurationPacket.INSTANCE);
                break;
            } else if (r.packet instanceof ClientboundDisconnectPacket p) {
                record("config", r.type + " " + json(p.reason()));
                return;
            } else {
                record("config", r.type + " " + (r.payload.length > 200 ? "sha=" + sha(r.payload) : text(r.packet)));
            }
        }
        play();
    }

    private void play() throws Exception {
        state = "play";
        GameProtocols.Context context = new GameProtocols.Context() {
            @Override
            public boolean hasInfiniteMaterials() {
                return false;
            }

            @Override
            public boolean canUseCommandBlocks() {
                return false;
            }
        };
        outbound = GameProtocols.SERVERBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access), context);
        inbound = GameProtocols.CLIENTBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access));
        socket.setSoTimeout(250);
        Map<String, String> chunks = new TreeMap<>();
        Map<String, Integer> counts = new TreeMap<>();
        boolean ordered = true;
        boolean loaded = false;
        PositionMoveRotation position = new PositionMoveRotation(Vec3.ZERO, Vec3.ZERO, 0, 0);
        long lastChunk = 0;
        long deadline = System.currentTimeMillis() + 90_000;
        int step = 0;
        long stepAt = 0;
        while (System.currentTimeMillis() < deadline) {
            Received r;
            try {
                r = read();
            } catch (SocketTimeoutException e) {
                r = null;
            }
            long now = System.currentTimeMillis();
            if (r != null) {
                Packet<?> packet = r.packet;
                counts.merge(r.type, 1, Integer::sum);
                if (r.type.equals("minecraft:chunk_batch_start")) ordered = false;
                if (packet instanceof ClientboundKeepAlivePacket p) {
                    send(new ServerboundKeepAlivePacket(p.getId()));
                } else if (packet instanceof ClientboundPingPacket p) {
                    send(new ServerboundPongPacket(p.getId()));
                } else if (packet instanceof ClientboundPlayerPositionPacket p) {
                    position = PositionMoveRotation.calculateAbsolute(position, p.change(), p.relatives());
                    record("play-move", text(position.position()) + " " + position.yRot() + " " + position.xRot());
                    send(new ServerboundAcceptTeleportationPacket(p.id(), position.position().x, position.position().y, position.position().z,
                        position.yRot(), position.xRot()));
                    send(new ServerboundMovePlayerPacket.PosRot(position.position(), position.yRot(), position.xRot(), false, false));
                    if (!loaded) {
                        send(new ServerboundPlayerLoadedPacket());
                        loaded = true;
                    }
                } else if (packet instanceof ClientboundChunkBatchFinishedPacket) {
                    send(new ServerboundChunkBatchReceivedPacket(64.0F));
                } else if (packet instanceof ClientboundLevelChunkWithLightPacket p) {
                    chunks.put(p.x() + "," + p.z(), sha(r.payload));
                    lastChunk = now;
                    int px = (int) Math.floor(position.position().x) >> 4;
                    int pz = (int) Math.floor(position.position().z) >> 4;
                    if (p.x() == px && p.z() == pz) chunkParts(p);
                } else if (packet instanceof ClientboundSystemChatPacket p && step > 0) {
                    record("play-chat", "system " + p.overlay() + " " + json(p.content()));
                } else if (packet instanceof ClientboundPlayerChatPacket p && step > 0) {
                    record("play-chat", "player " + p.sender() + " " + p.index() + " " + p.body().content() + " " + p.signature().isPresent()
                        + " " + p.unsignedContent().map(VanillaProtocolClient::json) + " " + text(p.filterMask()) + " "
                        + p.chatType().chatType().getRegisteredName() + " " + json(p.chatType().name()) + " "
                        + p.chatType().targetName().map(VanillaProtocolClient::json));
                } else if (packet instanceof ClientboundDisguisedChatPacket p && step > 0) {
                    record("play-chat", "disguised " + json(p.message()) + " " + p.chatType().chatType().getRegisteredName());
                } else if (packet instanceof ClientboundDisconnectPacket p) {
                    record("play", r.type + " " + json(p.reason()));
                    break;
                }
                if (ordered) {
                    record("play-start", r.type + " " + summary(r));
                }
            }
            // Once the chunks stop coming: chat, then /list, then leave.
            if (loaded && lastChunk > 0 && now - lastChunk > 3000 && step == 0) {
                record("play-chunks", chunks.size() + " " + chunks);
                step = 1;
                stepAt = now;
                send(new ServerboundChatPacket("hello from the parity client", Instant.now(), 0L, Optional.empty(),
                    new LastSeenMessages.Update(0, new BitSet(), (byte) 0)));
            } else if (step == 1 && now - stepAt > 1500) {
                step = 2;
                stepAt = now;
                send(new ServerboundChatCommandPacket("list"));
            } else if (step == 2 && now - stepAt > 1500) {
                record("play-counts", "chunks " + counts.getOrDefault("minecraft:level_chunk_with_light", 0));
                return;
            }
        }
        record("error", "timed out in play, step " + step + ", chunks " + chunks.size());
    }

    /** The player's chunk part by part, so a difference in its bytes can be told apart. */
    private void chunkParts(ClientboundLevelChunkWithLightPacket p) {
        var data = p.chunkData();
        for (var e : new TreeMap<>(data.getHeightmaps().entrySet().stream()
            .collect(java.util.stream.Collectors.toMap(x -> x.getKey().getSerializedName(), x -> x.getValue()))).entrySet()) {
            record("chunk-heightmap", e.getKey() + " " + java.util.Arrays.toString(e.getValue()).hashCode());
        }
        FriendlyByteBuf buf = data.getReadBuffer();
        byte[] all = new byte[buf.readableBytes()];
        buf.getBytes(buf.readerIndex(), all);
        var factory = PalettedContainerFactory.create(ParityData.access);
        int index = 0;
        while (buf.isReadable()) {
            int start = buf.readerIndex();
            LevelChunkSection section = new LevelChunkSection(factory);
            section.read(buf);
            StringBuilder states = new StringBuilder();
            for (int y = 0; y < 16; y++) for (int z = 0; z < 16; z++) for (int x = 0; x < 16; x++) {
                states.append(net.minecraft.world.level.block.Block.getId(section.getBlockState(x, y, z))).append(',');
            }
            StringBuilder biomes = new StringBuilder();
            for (int y = 0; y < 4; y++) for (int z = 0; z < 4; z++) for (int x = 0; x < 4; x++) {
                biomes.append(section.getNoiseBiome(x, y, z).getRegisteredName()).append(',');
            }
            byte[] bytes = new byte[buf.readerIndex() - start];
            buf.getBytes(start, bytes);
            record("chunk-section", index++ + " states=" + sha(states.toString().getBytes()) + " biomes=" + sha(biomes.toString().getBytes())
                + " count=" + (((bytes[0] & 0xFF) << 8) | (bytes[1] & 0xFF)) + " bytes=" + sha(bytes));
        }
        StringBuilder entities = new StringBuilder();
        data.forEachBlockEntityTag(p.x(), p.z(), (pos, type, tag) -> entities.append(pos.toShortString()).append(' ')
            .append(net.minecraft.core.registries.BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(type)).append(' ').append(tag).append(';'));
        record("chunk-block-entities", entities.toString());
        var light = p.lightData();
        record("chunk-light-masks", light.skyYMask() + " " + light.blockYMask() + " " + light.emptySkyYMask() + " " + light.emptyBlockYMask());
        StringBuilder sky = new StringBuilder();
        for (byte[] b : light.skyUpdates()) sky.append(sha(b)).append(' ');
        record("chunk-light-sky", sky.toString());
        StringBuilder block = new StringBuilder();
        for (byte[] b : light.blockUpdates()) block.append(sha(b)).append(' ');
        record("chunk-light-block", block.toString());
    }

    /** The first play packets: entity ids, seeds and times aside. */
    private String summary(Received r) {
        String t = r.type;
        if (t.equals("minecraft:login")) {
            return text(r.packet).replaceAll("playerId=-?\\d+", "playerId=_");
        }
        if (t.equals("minecraft:set_time") || t.equals("minecraft:keep_alive") || t.equals("minecraft:ping")) return "";
        if (t.contains("entity") || t.equals("minecraft:add_entity") || t.equals("minecraft:update_attributes")
            || t.equals("minecraft:player_info_update") || t.equals("minecraft:player_position")) {
            return "";
        }
        return r.payload.length > 200 ? "sha=" + sha(r.payload) : text(r.packet);
    }
}
