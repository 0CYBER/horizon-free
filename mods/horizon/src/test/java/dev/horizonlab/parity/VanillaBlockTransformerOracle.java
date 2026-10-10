package dev.horizonlab.parity;

import java.io.PrintWriter;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.LevelAccessor;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.core.component.BlockTransformer;

/**
 * Original oracle: the data-driven axe/hoe/shovel BlockTransformer entries.
 * "dump" lists every block state; a case line "transformer TAB face TAB
 * clicked TAB above" (states as "name|prop=value,...") prints the first
 * transform that yields a state and its settings, as BlockTransformer.transformBlock
 * would pick it.
 */
public final class VanillaBlockTransformerOracle {
    static String describe(BlockState state) {
        StringBuilder out = new StringBuilder(BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString()).append('|');
        List<Property<?>> props = new ArrayList<>(state.getProperties());
        props.sort(Comparator.comparing(Property::getName));
        boolean first = true;
        for (Property<?> p : props) {
            out.append(first ? "" : ",").append(p.getName()).append('=').append(name(state, p));
            first = false;
        }
        return out.toString();
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static String name(BlockState state, Property p) {
        return p.getName(state.getValue(p));
    }

    @SuppressWarnings({"unchecked", "rawtypes"})
    static BlockState parse(String text) {
        String[] parts = text.split("\\|", -1);
        Block block = BuiltInRegistries.BLOCK.getValue(Identifier.parse(parts[0]));
        BlockState state = block.defaultBlockState();
        if (!parts[1].isEmpty()) {
            for (String kv : parts[1].split(",")) {
                String[] pair = kv.split("=");
                Property p = block.getStateDefinition().getProperty(pair[0]);
                state = state.setValue(p, (Comparable) p.getValue(pair[1]).orElseThrow());
            }
        }
        return state;
    }

    static LevelAccessor level(BlockState clicked, BlockState above) {
        return (LevelAccessor) Proxy.newProxyInstance(LevelAccessor.class.getClassLoader(), new Class<?>[]{LevelAccessor.class}, (proxy, method, args) -> {
            if (method.getName().equals("getBlockState")) {
                BlockPos pos = (BlockPos) args[0];
                return pos.getY() == 0 ? clicked : pos.getY() == 1 && pos.getX() == 0 && pos.getZ() == 0 ? above : net.minecraft.world.level.block.Blocks.STONE.defaultBlockState();
            }
            throw new UnsupportedOperationException(method.getName());
        });
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        if (args[0].equals("dump")) {
            try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8))) {
                for (Block block : BuiltInRegistries.BLOCK) {
                    for (BlockState state : block.getStateDefinition().getPossibleStates()) out.println(describe(state));
                }
            }
            return;
        }
        var registry = ParityData.access.lookupOrThrow(Registries.BLOCK_TRANSFORMER);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8));
             var lines = Files.lines(Path.of(args[0]))) {
            for (String line : (Iterable<String>) lines::iterator) {
                String[] f = line.split("\t", -1);
                try {
                    if (f[0].equals("honeycomb")) {
                        out.println(net.minecraft.world.item.HoneycombItem.getWaxed(parse(f[2])).map(VanillaBlockTransformerOracle::describe).orElse("none"));
                        continue;
                    }
                    var transformer = registry.getOrThrow(ResourceKey.create(Registries.BLOCK_TRANSFORMER, Identifier.parse(f[0]))).value();
                    Direction face = Direction.byName(f[1]);
                    BlockState clicked = parse(f[2]);
                    LevelAccessor level = level(clicked, parse(f[3]));
                    String result = "none";
                    int index = 0;
                    for (var data : transformer.transforms()) {
                        if (!data.disallowedFaces().contains(face)) {
                            BlockState state = data.blockStateProvider().value().getOptionalState(level, RandomSource.create(0L), BlockPos.ZERO);
                            if (state != null) {
                                result = index + "\t" + describe(state) + "\t" + BuiltInRegistries.SOUND_EVENT.getKey(data.sound().value())
                                    + "\t" + data.particle().name().toLowerCase() + "\t"
                                    + data.loot().map(k -> k.identifier().toString()).orElse("-") + "\t" + data.dropStrategy().name().toLowerCase()
                                    + "\t" + data.updateFromNeighbors() + "\t" + data.transformType().name().toLowerCase() + "\t" + data.consumeOnUse()
                                    + "\t" + data.itemDamagePerUse();
                                break;
                            }
                        }
                        index++;
                    }
                    out.println(result);
                } catch (Throwable e) {
                    out.println("error " + e.getClass().getSimpleName() + " " + e.getMessage());
                }
            }
        }
    }
}
