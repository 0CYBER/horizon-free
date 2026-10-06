package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.stream.Collectors;
import net.minecraft.nbt.*;
import net.minecraft.world.level.chunk.status.ChunkType;
import net.minecraft.world.level.chunk.status.ChunkStatus;

/** Calls the pinned registry and status metadata/codec implementations. */
public final class VanillaChunkStatusOracle {
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        try(var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))){
            out.println("structure_distance "+ChunkStatus.class.getField("MAX_STRUCTURE_DISTANCE").getInt(null));
            for(var status:ChunkStatus.getStatusList()){
                out.println("status "+status.getIndex()+" "+status.getName()+" "+status.getParent().getName()+" "+(status.getChunkType()==ChunkType.LEVELCHUNK)+" "+status.heightmapsAfter().stream().map(Enum::name).collect(Collectors.joining(",")));
                for(var other:ChunkStatus.getStatusList())out.println("order "+status.isOrAfter(other)+" "+status.isAfter(other)+" "+status.isOrBefore(other)+" "+status.isBefore(other)+" "+ChunkStatus.max(status,other).getName());
            }
            for(var line:Files.readAllLines(Path.of(args[0]))){
                var row=JsonParser.parseString(line).getAsJsonArray();
                Tag tag=switch(row.get(0).getAsString()){
                    case "string"->StringTag.valueOf(row.get(1).getAsString());
                    case "int"->IntTag.valueOf(row.get(1).getAsInt());
                    case "list"->new ListTag();
                    case "compound"->new CompoundTag();
                    default->throw new AssertionError();
                };
                if(tag instanceof StringTag)out.println("lookup "+ChunkStatus.byName(row.get(1).getAsString()).getName());
                var result=ChunkStatus.CODEC.parse(NbtOps.INSTANCE,tag).result();
                if(result.isEmpty())out.println("decoded error");
                else {var status=result.get();var encoded=ChunkStatus.CODEC.encodeStart(NbtOps.INSTANCE,status).getOrThrow();out.println("decoded "+status.getIndex()+" "+((StringTag)encoded).value());}
            }
        }
    }
}
