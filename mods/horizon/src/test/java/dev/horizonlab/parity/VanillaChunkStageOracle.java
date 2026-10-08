package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.BelowZeroRetrogen;
import com.mojang.serialization.JsonOps;

/** Actual pre-light ProtoChunk writes and persisted-status transitions. */
public final class VanillaChunkStageOracle {
    private static final Map<String,ChunkStatus> STAGES=new HashMap<>();
    private static BelowZeroRetrogen upgrade(String target,JsonArray words) {
        var data=new JsonObject();data.addProperty("target_status",target);data.add("missing_bedrock",words);
        return BelowZeroRetrogen.CODEC.parse(JsonOps.INSTANCE,data).getOrThrow();
    }
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaHeightmapOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        STAGES.put("empty",ChunkStatus.EMPTY);STAGES.put("structure_starts",ChunkStatus.STRUCTURE_STARTS);STAGES.put("structure_references",ChunkStatus.STRUCTURE_REFERENCES);STAGES.put("biomes",ChunkStatus.BIOMES);STAGES.put("terrain",ChunkStatus.TERRAIN);STAGES.put("features",ChunkStatus.FEATURES);
        Path dir=Path.of(args[1]);
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();int min=f.get("min").getAsInt(),height=f.get("height").getAsInt();var cp=f.getAsJsonArray("chunk");int cx=cp.get(0).getAsInt(),cz=cp.get(1).getAsInt();
                var chunk=new ProtoChunk(new ChunkPos(cx,cz),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null);
                out.println("chunk "+cx+" "+cz+" "+min+" "+height);
                for(var value:f.getAsJsonArray("ops")) {
                    var a=value.getAsJsonArray();String op=a.get(0).getAsString();
                    switch(op) {
                        case "stage"->{String name=a.get(1).getAsString();chunk.setPersistedStatus(STAGES.get(name));out.println("stage "+name);}
                        case "upgrade"->{chunk.setBelowZeroRetrogen(a.get(1).isJsonNull()?null:upgrade(a.get(1).getAsString(),a.get(2).getAsJsonArray()));out.println("upgrade");}
                        case "upgrade_info"->{var data=chunk.getBelowZeroRetrogen();var bounds=chunk.getHeightAccessorForGeneration();out.println("upgrade_info "+chunk.isUpgrading()+" "+(data==null?"none":data.targetStatus().getName())+" "+bounds.getMinY()+" "+bounds.getHeight()+" "+chunk.getHighestGeneratedStatus().getName());}
                        case "replace_bedrock"->{BelowZeroRetrogen.replaceOldBedrock(chunk);out.println("replace_bedrock");}
                        case "mask"->{upgrade("full",a.get(1).getAsJsonArray()).applyBedrockMask(chunk);out.println("mask");}
                        case "fill"->{var states=a.get(1).getAsJsonArray();int salt=a.get(2).getAsInt();for(int y=0;y<height;y++)for(int z=0;z<16;z++)for(int x=0;x<16;x++)chunk.setBlockState(new BlockPos((cx<<4)+x,min+y,(cz<<4)+z),Block.stateById(states.get((x+3*y+5*z+salt)%states.size()).getAsInt()),3);out.println("fill");}
                        case "set"->{var p=new BlockPos(a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsInt());out.println("set "+Block.getId(chunk.setBlockState(p,Block.stateById(a.get(4).getAsInt()),3)));}
                        case "get"->out.println("get "+Block.getId(chunk.getBlockState(new BlockPos(a.get(1).getAsInt(),a.get(2).getAsInt(),a.get(3).getAsInt()))));
                        case "prime"->{int mask=a.get(1).getAsInt();var kinds=EnumSet.noneOf(Heightmap.Types.class);for(var kind:Heightmap.Types.values())if((mask&(1<<kind.ordinal()))!=0)kinds.add(kind);Heightmap.primeHeightmaps(chunk,kinds);out.println("prime "+mask);}
                        case "query"->{for(var kind:Heightmap.Types.values()) {
                            if(chunk.hasPrimedHeightmap(kind))out.println("height "+kind.name()+" "+chunk.getOrCreateHeightmapUnprimed(kind).getFirstAvailable(a.get(1).getAsInt()&15,a.get(2).getAsInt()&15));
                            else out.println("missing "+kind.name());
                        }}
                        case "dump"->{
                            for(var kind:Heightmap.Types.values()) {
                                if(chunk.hasPrimedHeightmap(kind)){out.println("map "+kind.name());for(long word:chunk.getOrCreateHeightmapUnprimed(kind).getRawData())out.println(Long.toUnsignedString(word,16));}
                                else out.println("missing "+kind.name());
                            }
                            for(int i=0;i<height/16;i++){out.println("section "+i);var section=chunk.getSection(i);for(int y=0;y<16;y++)for(int z=0;z<16;z++)for(int x=0;x<16;x++)out.println(Block.getId(section.getBlockState(x,y,z)));}
                        }
                        default->throw new AssertionError(op);
                    }
                }
            }
        }
    }
}
