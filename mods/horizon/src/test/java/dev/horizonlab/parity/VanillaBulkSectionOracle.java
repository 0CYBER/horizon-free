package dev.horizonlab.parity;
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.*;
import java.lang.reflect.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.placement.*;

/** Native feature placement into resident ProtoChunks through a world transport. */
public final class VanillaBulkSectionOracle {
    private static ProtoChunk chunk(Map<ChunkPos,ProtoChunk> chunks,int x,int z) {
        var chunk=chunks.get(new ChunkPos(x>>4,z>>4));if(chunk==null)throw new IllegalArgumentException("missing region chunk");return chunk;
    }
    private static int count(BulkSectionAccess bulk) throws Exception {
        var field=BulkSectionAccess.class.getDeclaredField("acquiredSections");field.setAccessible(true);return ((Map<?,?>)field.get(bulk)).size();
    }
    public static void main(String[] args) throws Exception {
        if(args[0].equals("export")){VanillaHeightmapOracle.main(args);return;}
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Path dir=Path.of(args[1]);
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);
        var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(dir.resolve("tags.json"))).getAsJsonObject());
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(var line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();int minimum=f.get("min").getAsInt(),height=f.get("height").getAsInt();
                var chunks=new TreeMap<ChunkPos,ProtoChunk>(Comparator.<ChunkPos>comparingInt(p->p.x()).thenComparingInt(p->p.z()));
                for(var value:f.getAsJsonArray("chunks")) {
                    var p=value.getAsJsonArray();var cp=new ChunkPos(p.get(0).getAsInt(),p.get(1).getAsInt());
                    var chunk=new ProtoChunk(cp,UpgradeData.EMPTY,LevelHeightAccessor.create(minimum,height),factory,null);
                    chunk.setPersistedStatus(ChunkStatus.TERRAIN);
                    if(f.get("prime").getAsBoolean())Heightmap.primeHeightmaps(chunk,EnumSet.allOf(Heightmap.Types.class));
                    chunks.put(cp,chunk);
                }
                var world=(LevelAccessor)Proxy.newProxyInstance(LevelAccessor.class.getClassLoader(),new Class<?>[]{LevelAccessor.class},(proxy,method,values)->switch(method.getName()) {
                    case "getMinY"->minimum;
                    case "getHeight"->height;
                    case "getChunk"->{var c=chunks.get(new ChunkPos((Integer)values[0],(Integer)values[1]));if(c==null)throw new IllegalArgumentException("missing chunk");yield c;}
                    default->{if(method.isDefault())yield InvocationHandler.invokeDefault(proxy,method,values);throw new AssertionError(method);}
                });
                var bulk=new BulkSectionAccess(world);
                try {
                    for(var value:f.getAsJsonArray("ops")) {
                        var op=value.getAsJsonArray();String kind=op.get(0).getAsString();
                        try {
                            if(kind.equals("reset")){bulk.close();bulk=new BulkSectionAccess(world);out.println("reset");}
                            else if(kind.equals("height")) {
                                int x=op.get(2).getAsInt(),z=op.get(3).getAsInt();
                                out.println("height "+(chunk(chunks,x,z).getHeight(Heightmap.Types.valueOf(op.get(1).getAsString()),x&15,z&15)+1));
                            } else {
                                var p=new BlockPos(op.get(1).getAsInt(),op.get(2).getAsInt(),op.get(3).getAsInt());
                                switch(kind) {
                                    case "get"->out.println("get "+Block.getId(bulk.getBlockState(p)));
                                    case "world"->out.println("world "+Block.getId(chunk(chunks,p.getX(),p.getZ()).getBlockState(p)));
                                    case "has"->out.println("has "+(bulk.getSection(p)!=null));
                                    case "set"->{var section=bulk.getSection(p);if(section==null)out.println("set none");else out.println("set "+Block.getId(section.setBlockState(p.getX()&15,p.getY()&15,p.getZ()&15,Block.stateById(op.get(4).getAsInt()),false)));}
                                    default->throw new AssertionError(kind);
                                }
                            }
                        } catch(IllegalArgumentException error){out.println("error");}
                        out.println("cache "+count(bulk));
                    }
                } finally {bulk.close();}
                for(var entry:chunks.entrySet()) {
                    var cp=entry.getKey();var chunk=entry.getValue();out.println("chunk "+cp.x()+" "+cp.z());
                    for(var kind:Heightmap.Types.values()) {
                        if(chunk.hasPrimedHeightmap(kind)){out.println("map "+kind.name());for(long word:chunk.getOrCreateHeightmapUnprimed(kind).getRawData())out.println(Long.toUnsignedString(word,16));}
                        else out.println("missing "+kind.name());
                    }
                    for(int i=0;i<height/16;i++){out.println("section "+i);var section=chunk.getSection(i);for(int y=0;y<16;y++)for(int z=0;z<16;z++)for(int x=0;x<16;x++)out.println(Block.getId(section.getBlockState(x,y,z)));}
                }
            }
        }
    }
}
