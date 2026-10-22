package dev.horizonlab.parity;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.serialization.JsonOps;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.ByteArrayInputStream;
import java.io.DataInputStream;
import java.io.EOFException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.PrintWriter;
import java.math.BigInteger;
import java.net.HttpURLConnection;
import java.net.InetSocketAddress;
import java.net.Socket;
import java.net.URI;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.KeyFactory;
import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.PrivateKey;
import java.security.spec.PKCS8EncodedKeySpec;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Base64;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.TimeUnit;
import java.util.zip.Deflater;
import java.util.zip.Inflater;
import javax.crypto.Cipher;
import javax.crypto.CipherInputStream;
import javax.crypto.CipherOutputStream;
import javax.crypto.SecretKey;
import net.minecraft.SharedConstants;
import net.minecraft.commands.arguments.ArgumentSignatures;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.ComponentSerialization;
import net.minecraft.network.chat.LastSeenMessages;
import net.minecraft.network.chat.LastSeenMessagesTracker;
import net.minecraft.network.chat.LocalChatSession;
import net.minecraft.network.chat.MessageSignature;
import net.minecraft.network.chat.MessageSignatureCache;
import net.minecraft.network.chat.PlayerChatMessage;
import net.minecraft.network.chat.RemoteChatSession;
import net.minecraft.network.chat.SignedMessageBody;
import net.minecraft.network.chat.SignedMessageChain;
import net.minecraft.network.chat.SignedMessageLink;
import net.minecraft.network.chat.SignedMessageValidator;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.common.ClientboundDisconnectPacket;
import net.minecraft.network.protocol.common.ClientboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ClientboundPingPacket;
import net.minecraft.network.protocol.common.ServerboundClientInformationPacket;
import net.minecraft.network.protocol.common.ServerboundKeepAlivePacket;
import net.minecraft.network.protocol.common.ServerboundPongPacket;
import net.minecraft.network.protocol.configuration.ClientboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ClientboundSelectKnownPacks;
import net.minecraft.network.protocol.configuration.ConfigurationProtocols;
import net.minecraft.network.protocol.configuration.ServerboundFinishConfigurationPacket;
import net.minecraft.network.protocol.configuration.ServerboundSelectKnownPacks;
import net.minecraft.network.protocol.game.ClientboundChunkBatchFinishedPacket;
import net.minecraft.network.protocol.game.ClientboundDisguisedChatPacket;
import net.minecraft.network.protocol.game.ClientboundLoginPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerChatPacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoRemovePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerInfoUpdatePacket;
import net.minecraft.network.protocol.game.ClientboundPlayerPositionPacket;
import net.minecraft.network.protocol.game.ClientboundSystemChatPacket;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.game.ServerboundAcceptTeleportationPacket;
import net.minecraft.network.protocol.game.ServerboundChatAckPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandPacket;
import net.minecraft.network.protocol.game.ServerboundChatCommandSignedPacket;
import net.minecraft.network.protocol.game.ServerboundChatPacket;
import net.minecraft.network.protocol.game.ServerboundChatSessionUpdatePacket;
import net.minecraft.network.protocol.game.ServerboundChunkBatchReceivedPacket;
import net.minecraft.network.protocol.game.ServerboundPlayerLoadedPacket;
import net.minecraft.network.protocol.handshake.ClientIntent;
import net.minecraft.network.protocol.handshake.ClientIntentionPacket;
import net.minecraft.network.protocol.handshake.HandshakeProtocols;
import net.minecraft.network.protocol.login.ClientboundHelloPacket;
import net.minecraft.network.protocol.login.ClientboundLoginCompressionPacket;
import net.minecraft.network.protocol.login.ClientboundLoginDisconnectPacket;
import net.minecraft.network.protocol.login.ClientboundLoginFinishedPacket;
import net.minecraft.network.protocol.login.LoginProtocols;
import net.minecraft.network.protocol.login.ServerboundHelloPacket;
import net.minecraft.network.protocol.login.ServerboundKeyPacket;
import net.minecraft.network.protocol.login.ServerboundLoginAcknowledgedPacket;
import net.minecraft.server.level.ClientInformation;
import net.minecraft.util.Crypt;
import net.minecraft.util.Signer;
import net.minecraft.world.entity.player.ProfileKeyPair;
import net.minecraft.world.entity.player.ProfilePublicKey;

/**
 * Secure chat seen by vanilla-codec clients: several players on one server,
 * signing their chat with profile keys that a mock of the Minecraft services
 * signed (the harness serves its public key as the services' key set).
 *
 * <p>{@code keys <dir>} writes the mock services' key pair: the private key
 * (PKCS#8) and {@code publickeys.json} as the services answer it. With a
 * host, a port, the private key, an output file and a scenario, it plays the
 * scenario. Each client signs and acknowledges exactly as
 * ClientPacketListener does (LocalChatSession, SignedMessageChain's encoder,
 * LastSeenMessagesTracker, MessageSignatureCache), and checks every player
 * chat it receives with SignedMessageValidator against the sender's session
 * from the player list. The transcript holds what each player received and
 * whether its signature checked out; keys, salts, times and signatures
 * differ between runs and are left out.
 */
public final class SecureChatClient {
    private static final List<String> LINES = new ArrayList<>();
    /** A command any player may run, whose reply only its sender gets (RandomCommand: value). */
    private static final String BARRIER = "random value 1..2";
    private static PrivateKey services;
    private static String sessionUrl;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        if (args[0].equals("keys")) {
            keys(Path.of(args[1]));
            return;
        }
        String host = args[0];
        int port = Integer.parseInt(args[1]);
        services = KeyFactory.getInstance("RSA").generatePrivate(new PKCS8EncodedKeySpec(Files.readAllBytes(Path.of(args[2]))));
        Path output = Path.of(args[3]);
        String scenario = args[4];
        sessionUrl = args.length > 5 ? args[5] : null;
        try {
            if (scenario.equals("offline")) offline(host, port);
            else online(host, port);
        } catch (Exception e) {
            StringBuilder line = new StringBuilder("error " + e);
            for (Throwable c = e.getCause(); c != null; c = c.getCause()) line.append(" / caused by ").append(c);
            LINES.add(line.toString());
        }
        try (PrintWriter w = new PrintWriter(Files.newBufferedWriter(output, StandardCharsets.UTF_8))) {
            for (String line : LINES) w.println(line);
        }
        System.exit(0);
    }

    private static void keys(Path dir) throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA");
        generator.initialize(2048);
        KeyPair pair = generator.generateKeyPair();
        Files.write(dir.resolve("services-private.der"), pair.getPrivate().getEncoded());
        JsonObject key = new JsonObject();
        key.addProperty("publicKey", Base64.getEncoder().encodeToString(pair.getPublic().getEncoded()));
        JsonArray list = new JsonArray();
        list.add(key);
        JsonObject set = new JsonObject();
        set.add("profilePropertyKeys", list);
        set.add("playerCertificateKeys", list);
        Files.writeString(dir.resolve("publickeys.json"), set.toString());
    }

    // ---- scenarios ----------------------------------------------------

    /** Offline mode: the server checks chat sessions but enforces nothing. */
    private static void offline(String host, int port) throws Exception {
        Client alice = Client.join(host, port, "Alice", true, null);
        Client bob = Client.join(host, port, "Bob", true, null);
        Client carol = Client.join(host, port, "Carol", false, null);
        List<Client> all = List.of(alice, bob, carol);
        settle(all, "joined");
        alice.chat("hello, signed");
        settle(all, "alice chats");
        bob.chat("seen it");
        settle(all, "bob chats with alice's message seen");
        carol.chat("no key here");
        settle(all, "carol chats unsigned");
        alice.signedCommand("say signed announcement", "message", "signed announcement");
        settle(all, "alice says");
        alice.signedCommand("me waves", "action", "waves");
        settle(all, "alice emotes");
        alice.signedCommand("msg Bob just between us", "message", "just between us");
        settle(all, "alice whispers to bob");
        carol.signedCommand("me shrugs", null, null);
        settle(all, "carol emotes, unsigned arguments");
        carol.command("say plain announcement");
        settle(all, "carol says through the unsigned packet");
        bob.signedCommand("say mismatch", "nothing", "mismatch");
        settle(all, "bob signs the wrong argument");
        bob.chat("after the mismatch");
        settle(all, "bob's chain after the mismatch");
        alice.tamperedChat("forged");
        settle(all, "alice's signature fails");
        alice.chat("after the forgery");
        settle(all, "alice's chain is broken");
        bob.send(new ServerboundChatAckPacket(50));
        settle(all, "bob acknowledges too much");
        leave(all);
    }

    /** Online mode with enforce-secure-profile: unsigned chat is refused. */
    private static void online(String host, int port) throws Exception {
        Client alice = Client.join(host, port, "Alice", true, null);
        Client carol = Client.join(host, port, "Carol", false, null);
        List<Client> all = List.of(alice, carol);
        settle(all, "joined");
        alice.chat("signed and enforced");
        settle(all, "alice chats");
        carol.chat("unsigned under enforcement");
        settle(all, "carol chats without a key");
        carol.command("me tries the unsigned packet");
        settle(all, "carol's unsigned command");
        carol.signedCommand("me tries without signatures", null, null);
        settle(all, "carol's command without signatures");
        // A key the services signed for another player.
        Client mallory = Client.join(host, port, "Mallory", true, UUID.nameUUIDFromBytes("someone else".getBytes()));
        settle(List.of(alice, carol, mallory), "mallory's borrowed key");
        leave(all);
        mallory.close();
    }

    /**
     * The players still connected leave one at a time, each once the next was
     * told the one before left (PlayerList.remove broadcasts the removal after
     * the "lost connection" line): the server logs the departures in this
     * order, not in the order its network threads notice the closed sockets.
     */
    private static void leave(List<Client> all) throws Exception {
        List<Client> clients = all.stream().filter(c -> !c.closed).toList();
        for (int i = 0; i < clients.size(); i++) {
            clients.get(i).close();
            if (i + 1 < clients.size()) clients.get(i + 1).awaitRemoved(clients.get(i));
        }
    }

    /**
     * Lets the server answer, then records what each player received, in player order. However slow the
     * server, a step ends once it handled everything sent so far: each player still connected runs a command
     * whose reply comes after it, twice (a chat's broadcast is a task the chat's own task queues), and has then
     * read whatever the server sent it before.
     */
    private static void settle(List<Client> clients, String step) throws Exception {
        Thread.sleep(1500);
        for (int round = 0; round < 2; round++) {
            for (Client c : clients) c.barrier();
        }
        LINES.add("step " + step);
        for (Client c : clients) c.drain();
    }

    private static String json(Component c) {
        return ComponentSerialization.CODEC.encodeStart(ParityData.access.createSerializationContext(JsonOps.INSTANCE), c).getOrThrow().toString();
    }

    /** A player: one connection, its chat session and what its client tracks. */
    private static final class Client {
        final String name;
        UUID id;
        Socket socket;
        DataInputStream in;
        OutputStream out;
        int threshold = -1;
        ProtocolInfo<?> inbound;
        ProtocolInfo<?> outbound;
        final BlockingQueue<Packet<?>> queue = new LinkedBlockingQueue<>();
        /** Replies to BARRIER, kept out of the queue. */
        final java.util.concurrent.Semaphore barriers = new java.util.concurrent.Semaphore(0);
        /** The server ended the connection (its reader stopped). */
        volatile boolean ended;
        volatile boolean closed;
        LocalChatSession session;
        SignedMessageChain.Encoder encoder = SignedMessageChain.Encoder.UNSIGNED;
        final LastSeenMessagesTracker lastSeen = new LastSeenMessagesTracker(20);
        final MessageSignatureCache cache = MessageSignatureCache.createDefault();
        int nextChatIndex;
        final Map<UUID, String> names = new LinkedHashMap<>();
        final Map<UUID, SignedMessageValidator> validators = new HashMap<>();
        boolean enforced;
        boolean loaded;
        net.minecraft.world.entity.PositionMoveRotation position = new net.minecraft.world.entity.PositionMoveRotation(
            net.minecraft.world.phys.Vec3.ZERO, net.minecraft.world.phys.Vec3.ZERO, 0, 0);

        Client(String name) {
            this.name = name;
        }

        void record(String line) {
            LINES.add(name + " " + line);
        }

        static Client join(String host, int port, String name, boolean signs, UUID keyOwner) throws Exception {
            Client c = new Client(name);
            c.connect(host, port, signs, keyOwner);
            return c;
        }

        void connect(String host, int port, boolean signs, UUID keyOwner) throws Exception {
            socket = new Socket();
            socket.connect(new InetSocketAddress(host, port), 10_000);
            socket.setSoTimeout(30_000);
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
                if (p instanceof ClientboundHelloPacket hello) {
                    // ClientHandshakePacketListenerImpl.handleHello: join through the session service.
                    SecretKey secret = Crypt.generateSecretKey();
                    String hash = new BigInteger(Crypt.digestData(hello.getServerId(), hello.getPublicKey(), secret)).toString(16);
                    joinSession(hash);
                    send(new ServerboundKeyPacket(secret, hello.getPublicKey(), hello.getChallenge()));
                    in = new DataInputStream(new CipherInputStream(socket.getInputStream(), Crypt.getCipher(Cipher.DECRYPT_MODE, secret)));
                    out = new CipherOutputStream(socket.getOutputStream(), Crypt.getCipher(Cipher.ENCRYPT_MODE, secret));
                } else if (p instanceof ClientboundLoginCompressionPacket compression) {
                    threshold = compression.getCompressionThreshold();
                } else if (p instanceof ClientboundLoginFinishedPacket finished) {
                    id = finished.gameProfile().id();
                    record("logged in");
                    send(ServerboundLoginAcknowledgedPacket.INSTANCE);
                    break;
                } else if (p instanceof ClientboundLoginDisconnectPacket d) {
                    record("refused " + json(d.reason()));
                    closed = true;
                    return;
                }
            }
            outbound = ConfigurationProtocols.SERVERBOUND;
            inbound = ConfigurationProtocols.CLIENTBOUND;
            send(new ServerboundClientInformationPacket(ClientInformation.createDefault()));
            while (true) {
                Packet<?> p = read();
                if (p instanceof ClientboundSelectKnownPacks packs) {
                    send(new ServerboundSelectKnownPacks(packs.knownPacks()));
                } else if (p instanceof ClientboundKeepAlivePacket k) {
                    send(new ServerboundKeepAlivePacket(k.getId()));
                } else if (p instanceof ClientboundPingPacket ping) {
                    send(new ServerboundPongPacket(ping.getId()));
                } else if (p instanceof ClientboundFinishConfigurationPacket) {
                    send(ServerboundFinishConfigurationPacket.INSTANCE);
                    break;
                } else if (p instanceof ClientboundDisconnectPacket d) {
                    record("disconnected " + json(d.reason()));
                    closed = true;
                    return;
                }
            }
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
            // ClientPacketListener.handleLogin: the session goes out once the
            // login packet is in (prepareKeyPair, setKeyPair).
            while (true) {
                Packet<?> p = read();
                if (p instanceof ClientboundLoginPacket login) {
                    enforced = login.enforcesSecureChat();
                    record("login online " + login.onlineMode() + " enforces " + login.enforcesSecureChat());
                    if (signs) startSession(keyOwner == null ? id : keyOwner);
                    break;
                }
            }
            Thread reader = new Thread(() -> {
                try {
                    while (!closed) {
                        Packet<?> p = read();
                        if (p instanceof ClientboundSystemChatPacket s
                            && s.content().getContents() instanceof net.minecraft.network.chat.contents.TranslatableContents t
                            && t.getKey().equals("commands.random.sample.success")) {
                            barriers.release();
                            continue;
                        }
                        queue.put(p);
                        // Nothing follows a disconnect but the closed socket.
                        if (p instanceof ClientboundDisconnectPacket) break;
                    }
                } catch (Exception e) {
                    if (!closed) queue.add(new ClientboundDisconnectPacket(Component.literal("connection lost: " + e)));
                }
                ended = true;
                barriers.release();
            }, name + " reader");
            reader.setDaemon(true);
            reader.start();
        }

        /** The session service's join, as the launcher's account would. */
        void joinSession(String serverHash) throws Exception {
            HttpURLConnection http = (HttpURLConnection) URI.create(sessionUrl + "/session/minecraft/join").toURL().openConnection();
            http.setRequestMethod("POST");
            http.setDoOutput(true);
            http.setRequestProperty("Content-Type", "application/json");
            JsonObject body = new JsonObject();
            body.addProperty("accessToken", "mock-token");
            body.addProperty("selectedProfile", name);
            body.addProperty("serverId", serverHash);
            http.getOutputStream().write(body.toString().getBytes(StandardCharsets.UTF_8));
            http.getResponseCode();
            http.disconnect();
        }

        /** A profile key pair the mock services signed for {@code owner}, and its session. */
        void startSession(UUID owner) throws Exception {
            KeyPairGenerator generator = KeyPairGenerator.getInstance("RSA");
            generator.initialize(2048);
            KeyPair pair = generator.generateKeyPair();
            Instant expires = Instant.now().plusSeconds(3600).truncatedTo(java.time.temporal.ChronoUnit.MILLIS);
            byte[] key = pair.getPublic().getEncoded();
            byte[] payload = ByteBuffer.allocate(24 + key.length).putLong(owner.getMostSignificantBits())
                .putLong(owner.getLeastSignificantBits()).putLong(expires.toEpochMilli()).put(key).array();
            byte[] signature = Signer.from(services, "SHA1withRSA").sign(payload);
            ProfilePublicKey publicKey = new ProfilePublicKey(new ProfilePublicKey.Data(expires, pair.getPublic(), signature));
            session = LocalChatSession.create(new ProfileKeyPair(pair.getPrivate(), publicKey, expires));
            encoder = session.createMessageEncoder(id);
            send(new ServerboundChatSessionUpdatePacket(session.asRemote().asData()));
        }

        /** ClientPacketListener.sendChat. */
        void chat(String content) throws Exception {
            Instant now = Instant.now();
            long salt = Crypt.SaltSupplier.getLong();
            LastSeenMessagesTracker.Update update = lastSeen.generateAndApplyUpdate();
            MessageSignature signature = encoder.pack(new SignedMessageBody(content, now, salt, update.lastSeen()));
            send(new ServerboundChatPacket(content, now, salt, Optional.ofNullable(signature), update.update()));
        }

        /** A chat whose signature doesn't match its text. */
        void tamperedChat(String content) throws Exception {
            Instant now = Instant.now();
            long salt = Crypt.SaltSupplier.getLong();
            LastSeenMessagesTracker.Update update = lastSeen.generateAndApplyUpdate();
            MessageSignature signature = encoder.pack(new SignedMessageBody(content + "!", now, salt, update.lastSeen()));
            send(new ServerboundChatPacket(content, now, salt, Optional.ofNullable(signature), update.update()));
        }

        /** ClientPacketListener.sendCommand for a command with one message argument (none: no signatures). */
        void signedCommand(String command, String argument, String value) throws Exception {
            Instant now = Instant.now();
            long salt = Crypt.SaltSupplier.getLong();
            LastSeenMessagesTracker.Update update = lastSeen.generateAndApplyUpdate();
            List<ArgumentSignatures.Entry> entries = new ArrayList<>();
            if (argument != null) {
                MessageSignature signature = encoder.pack(new SignedMessageBody(value, now, salt, update.lastSeen()));
                if (signature != null) entries.add(new ArgumentSignatures.Entry(argument, signature));
            }
            send(new ServerboundChatCommandSignedPacket(command, now, salt, new ArgumentSignatures(entries), update.update()));
        }

        void command(String command) throws Exception {
            send(new ServerboundChatCommandPacket(command));
        }

        /** Runs BARRIER and waits for its reply (30 s at most), which isn't recorded. */
        void barrier() throws Exception {
            if (closed || ended) return;
            barriers.drainPermits();
            try {
                send(new ServerboundChatCommandPacket(BARRIER));
            } catch (java.io.IOException e) {
                if (ended) return;
                throw e;
            }
            if (!barriers.tryAcquire(30, TimeUnit.SECONDS) && !ended) {
                LINES.add("error " + name + " got no reply to /" + BARRIER);
            }
        }

        /** Handles and records what arrived. */
        void drain() throws Exception {
            Packet<?> p;
            while ((p = queue.poll(50, TimeUnit.MILLISECONDS)) != null) handle(p);
        }

        void handle(Packet<?> packet) throws Exception {
            if (packet instanceof ClientboundKeepAlivePacket k) {
                send(new ServerboundKeepAlivePacket(k.getId()));
            } else if (packet instanceof ClientboundPingPacket ping) {
                send(new ServerboundPongPacket(ping.getId()));
            } else if (packet instanceof ClientboundPlayerPositionPacket pos) {
                position = net.minecraft.world.entity.PositionMoveRotation.calculateAbsolute(position, pos.change(), pos.relatives());
                send(new ServerboundAcceptTeleportationPacket(pos.id(), position.position().x, position.position().y,
                    position.position().z, position.yRot(), position.xRot()));
                if (!loaded) {
                    send(new ServerboundPlayerLoadedPacket());
                    loaded = true;
                }
            } else if (packet instanceof ClientboundChunkBatchFinishedPacket) {
                send(new ServerboundChunkBatchReceivedPacket(64.0F));
            } else if (packet instanceof ClientboundPlayerInfoUpdatePacket info) {
                for (ClientboundPlayerInfoUpdatePacket.Entry e : info.entries()) {
                    if (info.actions().contains(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER)) {
                        names.put(e.profileId(), e.profile().name());
                    }
                    if (info.actions().contains(ClientboundPlayerInfoUpdatePacket.Action.INITIALIZE_CHAT)) {
                        RemoteChatSession.Data data = e.chatSession();
                        String who = names.getOrDefault(e.profileId(), "?");
                        if (data != null) {
                            RemoteChatSession remote = new RemoteChatSession(data.sessionId(), new ProfilePublicKey(data.profilePublicKey()));
                            validators.put(e.profileId(), remote.createMessageValidator(ProfilePublicKey.EXPIRY_GRACE_PERIOD));
                            sessions.put(e.profileId(), data.sessionId());
                            record("session of " + who);
                        } else if (!info.actions().contains(ClientboundPlayerInfoUpdatePacket.Action.ADD_PLAYER)) {
                            record("session cleared for " + who);
                        }
                    }
                }
            } else if (packet instanceof ClientboundPlayerChatPacket chat) {
                playerChat(chat);
            } else if (packet instanceof ClientboundDisguisedChatPacket d) {
                record("disguised " + d.chatType().chatType().getRegisteredName() + " " + json(d.chatType().name()) + " "
                    + d.chatType().targetName().map(SecureChatClient::json) + " " + json(d.message()));
            } else if (packet instanceof ClientboundSystemChatPacket s) {
                record("system " + json(s.content()));
            } else if (packet instanceof ClientboundDisconnectPacket d) {
                record("disconnected " + json(d.reason()));
                closed = true;
            }
        }

        /** Waits (10 s at most) until the server removes `gone` from this player's list; records nothing else. */
        void awaitRemoved(Client gone) throws Exception {
            long until = System.nanoTime() + 10_000_000_000L;
            while (System.nanoTime() < until) {
                Packet<?> p = queue.poll(50, TimeUnit.MILLISECONDS);
                if (p instanceof ClientboundPlayerInfoRemovePacket r && r.profileIds().contains(gone.id)) return;
                if (p instanceof ClientboundKeepAlivePacket k) send(new ServerboundKeepAlivePacket(k.getId()));
                else if (p instanceof ClientboundPingPacket ping) send(new ServerboundPongPacket(ping.getId()));
            }
            LINES.add("error " + name + " was never told that " + gone.name + " left");
        }

        /** ClientPacketListener.handlePlayerChat, then markMessageAsProcessed. */
        void playerChat(ClientboundPlayerChatPacket p) {
            int expected = nextChatIndex++;
            String sender = names.getOrDefault(p.sender(), p.sender().toString());
            StringBuilder lastSeenPacked = new StringBuilder();
            for (MessageSignature.Packed packed : p.body().lastSeen().entries()) {
                lastSeenPacked.append(packed.fullSignature() != null ? "full" : "cached:" + packed.id()).append(',');
            }
            Optional<SignedMessageBody> body = p.body().unpack(cache);
            if (p.globalIndex() != expected || body.isEmpty()) {
                record("chat out of order or unknown signature: index " + p.globalIndex() + " expected " + expected);
                return;
            }
            MessageSignature signature = p.signature().orElse(null);
            cache.push(body.get(), signature);
            SignedMessageValidator validator = validators.get(p.sender());
            String verdict;
            if (validator != null) {
                PlayerChatMessage message = new PlayerChatMessage(
                    new SignedMessageLink(p.index(), p.sender(), sessionId(p.sender())), signature, body.get(),
                    p.unsignedContent().orElse(null), p.filterMask());
                verdict = validator.updateAndValidate(message) != null ? "verified" : "rejected";
            } else {
                verdict = signature == null ? (enforced ? "unsigned-rejected" : "unsigned") : "no-session";
            }
            record("chat #" + p.globalIndex() + " from " + sender + " link " + p.index() + " " + verdict + " '" + body.get().content()
                + "' seen [" + lastSeenPacked + "] " + body.get().lastSeen().entries().size() + " unsigned "
                + p.unsignedContent().map(SecureChatClient::json) + " " + p.chatType().chatType().getRegisteredName() + " "
                + json(p.chatType().name()) + " " + p.chatType().targetName().map(SecureChatClient::json));
            if (signature != null) lastSeen.addPending(signature, true);
        }

        private final Map<UUID, UUID> sessions = new HashMap<>();

        UUID sessionId(UUID sender) {
            return sessions.getOrDefault(sender, new UUID(0, 0));
        }

        // ---- framing ------------------------------------------------------

        synchronized void send(Packet<?> packet) throws Exception {
            ByteBuf body = Unpooled.buffer();
            encode(body, packet);
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
            // A server that disconnected the player may close before this
            // write: what it sent last is the disconnect, which the reader
            // records, not the broken pipe.
            if (broken) return;
            try {
                out.write(bytes);
                out.flush();
            } catch (java.io.IOException e) {
                broken = true;
            }
        }

        private boolean broken;

        @SuppressWarnings({"unchecked", "rawtypes"})
        private void encode(ByteBuf body, Packet<?> packet) {
            ((ProtocolInfo) outbound).codec().encode(body, packet);
        }

        Packet<?> read() throws Exception {
            int length = readVarInt(in);
            byte[] frame = new byte[length];
            in.readFully(frame);
            byte[] raw = frame;
            if (threshold >= 0) {
                DataInputStream f = new DataInputStream(new ByteArrayInputStream(frame));
                int dataLength = readVarInt(f);
                byte[] rest = f.readAllBytes();
                if (dataLength == 0) {
                    raw = rest;
                } else {
                    Inflater inflater = new Inflater();
                    inflater.setInput(rest);
                    raw = new byte[dataLength];
                    inflater.inflate(raw);
                    inflater.end();
                }
            }
            return inbound.codec().decode(Unpooled.wrappedBuffer(raw));
        }

        void close() throws Exception {
            closed = true;
            socket.close();
        }
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
}
