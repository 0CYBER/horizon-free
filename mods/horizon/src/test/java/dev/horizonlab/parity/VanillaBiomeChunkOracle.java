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

/** Original transport calling vanilla section-atomic biome fill and chunk reads. */
public final class VanillaBiomeChunkOracle {
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var ids=new IdMapper<Holder<Biome>>();
        for(int i=0;i<512;i++)ids.add(Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},ResourceKey.create(Registries.BIOME,Identifier.parse("horizon:biome_"+i))));
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
                    try { chunk.fillBiomesFromNoise((x,y,z)->{
                        out.println("resolve "+x+" "+y+" "+z);int previous=calls[0]++;
                        if(previous==op.get(2).getAsInt())throw new IllegalStateException("synthetic provider failure");
                        return ids.byId(Math.floorMod(x*31+y*7+z*17+op.get(0).getAsInt(),op.get(1).getAsInt()));
                    });}catch(IllegalStateException error){ok=false;}
                    out.println("fill "+(ok?"ok":"error")+" "+calls[0]);
                    for(int section=0;section<height/16;section++) {
                        out.println("section "+section);var data=chunk.getSection(section);
                        for(int y=0;y<4;y++)for(int z=0;z<4;z++){out.print("biomes");for(int x=0;x<4;x++)out.print(" "+ids.getId(data.getNoiseBiome(x,y,z)));out.println();}
                    }
                    for(var q:f.getAsJsonArray("queries")){var a=q.getAsJsonArray();out.println("query "+ids.getId(chunk.getNoiseBiome(a.get(0).getAsInt(),a.get(1).getAsInt(),a.get(2).getAsInt())));}
                }
            }
        }
    }
}
