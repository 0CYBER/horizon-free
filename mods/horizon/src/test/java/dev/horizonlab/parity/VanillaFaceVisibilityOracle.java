package dev.horizonlab.parity;

import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.*;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;

/** Imports definition kinds and raw geometry; compares actual skipRendering and
 * Block.shouldRenderFace on the unmodified native state registry. */
public final class VanillaFaceVisibilityOracle {
    private static final Gson JSON=new GsonBuilder().serializeNulls().create();
    private static String rule(Block block) throws Exception {
        for(Class<?> type=block.getClass();type!=null;type=type.getSuperclass()) {
            try {
                type.getDeclaredMethod("skipRendering",BlockState.class,BlockState.class,Direction.class);
                return switch(type.getSimpleName()) {
                    case "BlockBehaviour" -> "never";
                    case "HalfTransparentBlock", "PowderSnowBlock" -> "same_block";
                    case "IronBarsBlock" -> "bars";
                    case "LeavesBlock" -> "leaves";
                    case "LiquidBlock" -> "liquid";
                    case "MangroveRootsBlock" -> "mangrove_roots";
                    default -> throw new AssertionError("Unhandled skip-rendering owner "+type);
                };
            } catch(NoSuchMethodException ignored) {}
        }
        throw new AssertionError("Missing skip-rendering method");
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var directory=Path.of(args[1]);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(directory.resolve("tags.json"))).getAsJsonObject());
        if(args[0].equals("export")) {
            VanillaCollisionOracle.main(new String[]{"generate",args[1]});
            try(var output=new DataOutputStream(Files.newOutputStream(directory.resolve("occlusion.nbt")))) {
                NbtIo.write(VanillaCollisionOracle.occlusionDefinitions(),output);
            }
            var definitions=new TreeMap<String,Object>();
            for(var block:BuiltInRegistries.BLOCK) {
                var state=block.defaultBlockState();var ids=new ArrayList<Integer>();
                for(var value:block.getStateDefinition().getPossibleStates())ids.add(Block.getId(value));
                definitions.put(BuiltInRegistries.BLOCK.getKey(block).toString(),Map.of("rule",rule(block),
                    "can_occlude",state.canOcclude(),"bars_tag",state.is(BlockTags.BARS),"default",Block.getId(state),"states",ids));
            }
            Files.writeString(directory.resolve("definitions.json"),JSON.toJson(definitions));return;
        }
        try(var inputs=Files.newBufferedReader(directory.resolve("pairs.txt"));var output=Files.newBufferedWriter(directory.resolve(args[0]+".txt"))) {
            for(String line;(line=inputs.readLine())!=null;) {
                var values=line.split(" ");int a=Integer.parseInt(values[0]),b=Integer.parseInt(values[1]);
                var state=Block.stateById(a);var neighbor=Block.stateById(b);
                for(boolean cutout:new boolean[]{false,true}) {
                    LeavesBlock.setCutoutLeaves(cutout);int skip=0,visible=0;
                    for(var direction:Direction.values()) {
                        if(state.skipRendering(neighbor,direction))skip|=1<<direction.ordinal();
                        if(Block.shouldRenderFace(state,neighbor,direction))visible|=1<<direction.ordinal();
                    }
                    output.write(a+" "+b+" "+cutout+" "+skip+" "+visible);output.newLine();
                }
            }
        }
    }
}
