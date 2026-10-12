package dev.horizonlab.parity;

import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.BufferedReader;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HexFormat;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.configuration.ConfigurationProtocols;
import net.minecraft.network.protocol.game.ClientboundLevelChunkWithLightPacket;
import net.minecraft.network.protocol.game.GameProtocols;
import net.minecraft.network.protocol.login.LoginProtocols;
import net.minecraft.network.protocol.status.StatusProtocols;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainerFactory;

/**
 * Checks a capture written by tools/net_capture.py against vanilla's own
 * packet codecs: every clientbound packet a server sent (login,
 * configuration and play) must decode with vanilla's StreamCodec, consume
 * its whole payload, and encode back to the same bytes, so the bytes are
 * exactly what vanilla would have written for that packet. Chunk data is
 * decoded section by section and the stored block counts are checked
 * against a recount. Prints one line per packet:
 *   conn dir state name ok|ERROR|REENCODE|SECTIONS detail
 * and a summary per packet type at the end.
 */
public final class VanillaPacketCodecOracle {
    public static void main(String[] args) throws Exception {
        ParityData.init();
        Path capture = Path.of(args[0]);
        boolean verbose = args.length > 2 && args[2].equals("--verbose");
        var play = GameProtocols.CLIENTBOUND_TEMPLATE.bind(RegistryFriendlyByteBuf.decorator(ParityData.access));
        var factory = PalettedContainerFactory.create(ParityData.access);
        Field nonEmpty = LevelChunkSection.class.getDeclaredField("nonEmptyBlockCount");
        Field fluids = LevelChunkSection.class.getDeclaredField("fluidCount");
        nonEmpty.setAccessible(true);
        fluids.setAccessible(true);
        Map<String, int[]> totals = new TreeMap<>();
        int failures = 0;
        try (BufferedReader in = Files.newBufferedReader(capture);
             PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            String line;
            while ((line = in.readLine()) != null) {
                if (line.isBlank()) continue;
                JsonObject e = JsonParser.parseString(line).getAsJsonObject();
                if (!e.has("name") || !e.get("dir").getAsString().equals("s2c")) continue;
                String state = e.get("state").getAsString();
                String name = e.get("name").getAsString();
                String key = state + "/" + name;
                int[] t = totals.computeIfAbsent(key, k -> new int[2]);
                t[0]++;
                if (!e.has("data")) {
                    out.println(e.get("conn") + " s2c " + key + " SKIPPED payload not captured");
                    t[1]++;
                    failures++;
                    continue;
                }
                byte[] payload = HexFormat.of().parseHex(e.get("data").getAsString());
                ProtocolInfo<?> protocol = switch (state) {
                    case "status" -> StatusProtocols.CLIENTBOUND;
                    case "login" -> LoginProtocols.CLIENTBOUND;
                    case "configuration" -> ConfigurationProtocols.CLIENTBOUND;
                    case "play" -> play;
                    default -> null;
                };
                if (protocol == null) continue;
                ByteBuf original = Unpooled.buffer();
                new FriendlyByteBuf(original).writeVarInt(e.get("id").getAsInt());
                original.writeBytes(payload);
                byte[] framed = new byte[original.readableBytes()];
                original.getBytes(0, framed);
                String verdict;
                String detail = "";
                try {
                    @SuppressWarnings("unchecked")
                    var codec = (net.minecraft.network.codec.StreamCodec<ByteBuf, Packet<?>>) (Object) protocol.codec();
                    Packet<?> packet = codec.decode(original);
                    if (original.readableBytes() != 0) {
                        verdict = "ERROR";
                        detail = original.readableBytes() + " bytes left after decoding " + packet;
                    } else {
                        ByteBuf again = Unpooled.buffer();
                        codec.encode(again, packet);
                        byte[] encoded = new byte[again.readableBytes()];
                        again.getBytes(0, encoded);
                        if (!java.util.Arrays.equals(encoded, framed)) {
                            verdict = "REENCODE";
                            detail = firstDifference(framed, encoded) + " " + abbreviate(packet.toString());
                        } else if (packet instanceof ClientboundLevelChunkWithLightPacket chunk) {
                            String problem = checkSections(chunk, factory, nonEmpty, fluids);
                            verdict = problem == null ? "ok" : "SECTIONS";
                            detail = problem == null ? "" : problem;
                        } else {
                            verdict = "ok";
                            if (verbose) detail = abbreviate(packet.toString());
                        }
                    }
                } catch (Exception ex) {
                    verdict = "ERROR";
                    detail = ex.toString();
                }
                if (!verdict.equals("ok")) {
                    t[1]++;
                    failures++;
                }
                if (!verdict.equals("ok") || verbose) {
                    out.println(e.get("conn") + " s2c " + key + " " + verdict + " " + detail);
                }
            }
            out.println("# packet type, count, failures");
            for (var entry : totals.entrySet()) {
                out.println("# " + entry.getKey() + " " + entry.getValue()[0] + " " + entry.getValue()[1]);
            }
            out.println("# failures " + failures);
        }
        System.exit(failures == 0 ? 0 : 1);
    }

    static String checkSections(ClientboundLevelChunkWithLightPacket chunk, PalettedContainerFactory factory,
                                Field nonEmpty, Field fluids) throws Exception {
        FriendlyByteBuf buf = chunk.chunkData().getReadBuffer();
        int index = 0;
        while (buf.isReadable()) {
            int start = buf.readerIndex();
            short storedBlocks = buf.getShort(start);
            short storedFluids = buf.getShort(start + 2);
            LevelChunkSection section = new LevelChunkSection(factory);
            section.read(buf);
            section.recalcBlockCounts();
            int blocks = (short) (int) nonEmpty.get(section);
            int fluidCount = (short) (int) fluids.get(section);
            if (blocks != storedBlocks || fluidCount != storedFluids) {
                return "section " + index + " of " + chunk.x() + "," + chunk.z() + " stores " + storedBlocks + " blocks/"
                    + storedFluids + " fluids, recount " + blocks + "/" + fluidCount;
            }
            index++;
        }
        if (index != 24 && index != 16) return index + " sections in " + chunk.x() + "," + chunk.z();
        return null;
    }

    static String firstDifference(byte[] a, byte[] b) {
        int n = Math.min(a.length, b.length);
        for (int i = 0; i < n; i++) {
            if (a[i] != b[i]) return "differs at byte " + i + " of " + a.length + " (vanilla writes " + b.length + ")";
        }
        return "lengths " + a.length + " vs vanilla " + b.length;
    }

    static String abbreviate(String s) {
        s = s.replace('\n', ' ');
        return s.length() > 400 ? s.substring(0, 400) + "..." : s;
    }
}
