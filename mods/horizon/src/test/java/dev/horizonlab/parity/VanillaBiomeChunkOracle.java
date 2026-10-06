package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.BelowZeroRetrogen;
import com.mojang.serialization.JsonOps;

/** Original transport calling vanilla section-atomic biome fill and chunk reads. */
public final class VanillaBiomeChunkOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var ids=new IdMapper<Holder<Biome>>();
        for(int i=0;i<512;i++) {
            String key=switch(i){case 0->"minecraft:lush_caves";case 1->"minecraft:dripstone_caves";case 2->"minecraft:deep_dark";case 3->"horizon:lush_caves";default->"horizon:biome_"+i;};
            // No biome properties are accessed; direct holders have no registry key.
            ids.add(i==511?Holder.direct(null):Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse(key))));
        }
        var upgrade=BelowZeroRetrogen.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString("{\"target_status\":\"minecraft:features\"}")).getOrThrow();
        Path dir=Path.of(args[1]);
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt"),StandardCharsets.UTF_8))) {
            for(String line:Files.readAllLines(dir.resolve("inputs.txt"))) {
                var f=JsonParser.parseString(line).getAsJsonObject();int min=f.get("min").getAsInt(),height=f.get("height").getAsInt();
                var factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),ids.byId(f.get("default").getAsInt()),null);
                var bounds=new int[]{min,min+height-1};
                var chunk=new ProtoChunk(new ChunkPos(f.get("cx").getAsInt(),f.get("cz").getAsInt()),UpgradeData.EMPTY,LevelHeightAccessor.create(min,height),factory,null){@Override public LevelHeightAccessor getHeightAccessorForGeneration(){return LevelHeightAccessor.create(bounds[0],bounds[1]-bounds[0]+1);}};
                chunk.setPersistedStatus(net.minecraft.world.level.chunk.status.ChunkStatus.BIOMES);
                for(var value:f.getAsJsonArray("ops")) {
                    var op=value.getAsJsonArray();bounds[0]=op.get(3).getAsInt();bounds[1]=op.get(4).getAsInt();var calls=new int[1];boolean ok=true;
                    try { BiomeResolver candidate=(x,y,z)->{
                        out.println("resolve "+x+" "+y+" "+z);int previous=calls[0]++;
                        if(previous==op.get(2).getAsInt())throw new IllegalStateException("synthetic provider failure");
                        return ids.byId(Math.floorMod(x*31+y*7+z*17+op.get(0).getAsInt(),op.get(1).getAsInt()));
                    };
                        chunk.setBelowZeroRetrogen(op.size()>5&&op.get(5).getAsBoolean()?upgrade:null);
                        var resolver=BelowZeroRetrogen.getBiomeResolver(candidate,chunk);
                        chunk.setBelowZeroRetrogen(op.size()>6&&op.get(6).getAsBoolean()?upgrade:null);
                        chunk.fillBiomesFromNoise(resolver);
                    }catch(IllegalStateException error){ok=false;}
                    out.println("fill "+(ok?"ok":"error")+" "+calls[0]);
                    for(int section=0;section<height/16;section++) {
                        out.println("section "+section);var data=chunk.getSection(section);
                        for(int y=0;y<4;y++)for(int z=0;z<4;z++){out.print("biomes");for(int x=0;x<4;x++)out.print(" "+ids.getId(data.getNoiseBiome(x,y,z)));out.println();}
                    }
                    for(var q:f.getAsJsonArray("queries")){var a=q.getAsJsonArray();out.println("query "+ids.getId(chunk.getNoiseBiome(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt())));}
                }
                if(f.has("resolver_queries"))for(var value:f.getAsJsonArray("resolver_queries")) {
                    var q=value.getAsJsonArray();
                    chunk.setBelowZeroRetrogen(q.get(4).getAsBoolean()?upgrade:null);
                    var resolver=BelowZeroRetrogen.getBiomeResolver((x,y,z)->{
                        out.println("resolve_query "+x+" "+y+" "+z);
                        if(q.get(0).getAsInt()<0)throw new IllegalStateException("synthetic candidate failure");
                        return ids.byId(q.get(0).getAsInt());
                    },chunk);
                    chunk.setBelowZeroRetrogen(q.get(5).getAsBoolean()?upgrade:null);
                    try {out.println("resolver_query "+ids.getId(resolver.getNoiseBiome(q.get(1).getAsInt(),q.get(2).getAsInt(),q.get(3).getAsInt())));}
                    catch(IllegalStateException error){out.println("resolver_query error");}
                }
            }
        }
    }
}
