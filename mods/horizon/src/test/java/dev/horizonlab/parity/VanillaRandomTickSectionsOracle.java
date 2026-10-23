package dev.horizonlab.parity;

import java.io.*;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainerFactory;

/**
 * Original oracle for which sections of a chunk random-tick and where: the
 * real LevelChunkSection block counts (setBlockState, and recalcBlockCounts
 * for a section built from a filled container, as a loaded one is) and
 * isRandomlyTicking, with the block part of ServerLevel.tickChunk's section
 * loop around them (the precipitation draws included, on a level random
 * seeded 0) and the real Level.getBlockRandomPos drawing the positions.
 *
 * Each case line is "randValue speed op...", ops being "s:section:index:state"
 * (setBlockState, index y << 8 | z << 4 | x), "f:section:a:b:m" (a section
 * loaded from a container holding b where index % m == m - 1, a elsewhere) and
 * "t" (one tick). Prints, per tick, every drawn position with whether its
 * block and its fluid tick randomly, then the final randValue.
 */
public final class VanillaRandomTickSectionsOracle {
    static final int SECTIONS = 24;
    static final int MIN_Y = -64;

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var factory = PalettedContainerFactory.create(ParityData.access);
        Level level = ParityData.level;
        Field randValue = Level.class.getDeclaredField("randValue");
        randValue.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (var line : (Iterable<String>) lines::iterator) {
                if (line.startsWith("#")) continue;
                String[] words = line.trim().split(" ");
                randValue.setInt(level, Integer.parseInt(words[0]));
                int speed = Integer.parseInt(words[1]);
                RandomSource random = RandomSource.create(0);
                LevelChunkSection[] sections = new LevelChunkSection[SECTIONS];
                for (int i = 0; i < SECTIONS; i++) sections[i] = new LevelChunkSection(factory);
                StringBuilder result = new StringBuilder();
                for (int w = 2; w < words.length; w++) {
                    String[] op = words[w].split(":");
                    switch (op[0]) {
                        case "s" -> {
                            int index = Integer.parseInt(op[2]);
                            sections[Integer.parseInt(op[1])].setBlockState(index & 15, index >> 8, index >> 4 & 15,
                                state(op[3]));
                        }
                        case "f" -> {
                            BlockState a = state(op[2]), b = state(op[3]);
                            int m = Integer.parseInt(op[4]);
                            var states = factory.createForBlockStates();
                            for (int index = 0; index < 4096; index++) {
                                states.getAndSetUnchecked(index & 15, index >> 8, index >> 4 & 15,
                                    index % m == m - 1 ? b : a);
                            }
                            sections[Integer.parseInt(op[1])] = new LevelChunkSection(states, factory.createForBiomes());
                        }
                        case "t" -> {
                            result.append('|');
                            // ServerLevel.tickChunk, for a chunk at 0, 0. Its
                            // precipitation positions use randValue too.
                            for (int i = 0; i < speed; i++) {
                                if (random.nextInt(48) == 0) level.getBlockRandomPos(0, 0, 0, 15);
                            }
                            if (speed <= 0) continue;
                            for (int s = 0; s < SECTIONS; s++) {
                                LevelChunkSection section = sections[s];
                                if (!section.isRandomlyTicking()) continue;
                                int minY = MIN_Y + s * 16;
                                for (int i = 0; i < speed; i++) {
                                    BlockPos pos = level.getBlockRandomPos(0, minY, 0, 15);
                                    BlockState state = section.getBlockState(pos.getX(), pos.getY() - minY, pos.getZ());
                                    result.append(' ').append(pos.getX()).append(',').append(pos.getY()).append(',')
                                        .append(pos.getZ()).append(',')
                                        .append(state.isRandomlyTicking() ? 'B' : '-')
                                        .append(state.getFluidState().isRandomlyTicking() ? 'F' : '-');
                                }
                            }
                        }
                        default -> throw new IllegalArgumentException(words[w]);
                    }
                }
                out.println(result.append(" | ").append(randValue.getInt(level)));
            }
        }
        System.exit(0);
    }

    static BlockState state(String id) {
        return Block.BLOCK_STATE_REGISTRY.byId(Integer.parseInt(id));
    }
}
