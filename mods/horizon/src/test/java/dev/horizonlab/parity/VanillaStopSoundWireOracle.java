package dev.horizonlab.parity;

import com.google.gson.*;
import io.netty.buffer.Unpooled;
import java.nio.file.*;
import java.util.*;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.game.ClientboundStopSoundPacket;
import net.minecraft.resources.Identifier;
import net.minecraft.sounds.SoundSource;

/** Every source/event selection combination through the native stream codec. */
public final class VanillaStopSoundWireOracle {
    public static void main(String[] args) throws Exception {
        var out = new ArrayList<String>();
        for (int source = -1; source < SoundSource.values().length; source++) {
            for (String sound : new String[]{null, "minecraft:entity.pig.ambient", "example:custom"}) {
                var packet = new ClientboundStopSoundPacket(sound == null ? null : Identifier.parse(sound), source < 0 ? null : SoundSource.values()[source]);
                var buf = new FriendlyByteBuf(Unpooled.buffer());
                ClientboundStopSoundPacket.STREAM_CODEC.encode(buf, packet);
                byte[] bytes = new byte[buf.readableBytes()]; buf.readBytes(bytes); buf.release();
                var result = new JsonObject(); result.addProperty("wire", HexFormat.of().formatHex(bytes));
                result.add("source", source < 0 ? JsonNull.INSTANCE : new JsonPrimitive(source));
                result.add("sound", sound == null ? JsonNull.INSTANCE : new JsonPrimitive(sound));
                out.add(new Gson().toJson(result));
            }
        }
        Files.write(Path.of(args[0]), out);
    }
}
