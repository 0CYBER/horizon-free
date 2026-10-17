package dev.horizonlab.parity;

import io.netty.buffer.Unpooled;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.Map;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.game.ClientboundGameRuleValuesPacket;
import net.minecraft.network.protocol.game.ServerboundChangeDifficultyPacket;
import net.minecraft.network.protocol.game.ServerboundLockDifficultyPacket;
import net.minecraft.network.protocol.game.ServerboundSetGameRulePacket;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.Difficulty;

/** Encode the same settings through the pinned game's own StreamCodecs. */
public final class VanillaWorldOptionsOracle {
    public static void main(String[] args) throws Exception {
        var output = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            String[] fields = line.split("\t");
            var buffer = new FriendlyByteBuf(Unpooled.buffer());
            switch (fields[0]) {
                case "difficulty" -> ServerboundChangeDifficultyPacket.STREAM_CODEC.encode(buffer,
                    new ServerboundChangeDifficultyPacket(Difficulty.byId(Integer.parseInt(fields[1]))));
                case "lock" -> ServerboundLockDifficultyPacket.STREAM_CODEC.encode(buffer,
                    new ServerboundLockDifficultyPacket(Boolean.parseBoolean(fields[1])));
                case "set", "values" -> {
                    var entries = fields.length == 1 ? List.<ServerboundSetGameRulePacket.Entry>of()
                        : List.of(new ServerboundSetGameRulePacket.Entry(ResourceKey.create(
                            Registries.GAME_RULE, Identifier.withDefaultNamespace(fields[1])), fields[2]));
                    if (fields[0].equals("set")) {
                        ServerboundSetGameRulePacket.STREAM_CODEC.encode(buffer,
                            new ServerboundSetGameRulePacket(entries));
                    } else {
                        var values = entries.isEmpty() ? Map.<ResourceKey<net.minecraft.world.level.gamerules.GameRule<?>>, String>of()
                            : Map.of(entries.getFirst().gameRuleKey(), entries.getFirst().value());
                        ClientboundGameRuleValuesPacket.STREAM_CODEC.encode(buffer,
                            new ClientboundGameRuleValuesPacket(values));
                    }
                }
                default -> throw new IllegalArgumentException(line);
            }
            byte[] bytes = new byte[buffer.readableBytes()];
            buffer.readBytes(bytes);
            output.add(HexFormat.of().formatHex(bytes));
            buffer.release();
        }
        Files.write(Path.of(args[1]), output);
    }
}
