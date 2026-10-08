package dev.horizonlab.parity;
import com.google.gson.*;import java.nio.file.*;import java.util.*;
import net.minecraft.core.*;import net.minecraft.core.registries.*;import net.minecraft.tags.*;import net.minecraft.world.level.block.*;
/** Run actual entity immersion with registered fluid heights, flows and support. */
public final class VanillaRegisteredEntityFluidOracle {
    public static void main(String[] args)throws Exception {
        if(args[0].equals("export")){VanillaRegistryFluidOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        var natural=new ArrayList<Holder<Block>>();var all=new ArrayList<Holder<Block>>();for(var block:BuiltInRegistries.BLOCK){all.add(block.builtInRegistryHolder());if(block.defaultBlockState().is(BlockTags.BLOCKS_FLUID_FLOW))natural.add(block.builtInRegistryHolder());}
        int[] old={-1};VanillaEntityFluidOracle.setup=op->{int tag=op.get("block_tag").getAsInt();if(old[0]!=tag){BuiltInRegistries.BLOCK.prepareTagReload(new TagLoader.LoadResult<>(Registries.BLOCK,Map.of(BlockTags.BLOCKS_FLUID_FLOW,tag==0?natural:tag==1?List.<Holder<Block>>of():all))).apply();old[0]=tag;}};
        VanillaEntityFluidOracle.main(args);
    }
}
