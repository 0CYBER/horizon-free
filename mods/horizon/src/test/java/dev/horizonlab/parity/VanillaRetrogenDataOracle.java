package dev.horizonlab.parity;
import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.nbt.*;
import net.minecraft.world.level.levelgen.BelowZeroRetrogen;

/** Original typed-NBT transport for the native retrogen codec and hole queries. */
public final class VanillaRetrogenDataOracle {
    private static Tag tag(JsonElement value) {
        var row=value.getAsJsonArray();String kind=row.get(0).getAsString();var v=row.get(1);
        return switch(kind){
            case "end"->EndTag.INSTANCE;
            case "byte"->ByteTag.valueOf(v.getAsByte());
            case "short"->ShortTag.valueOf(v.getAsShort());
            case "int"->IntTag.valueOf(v.getAsInt());
            case "long"->LongTag.valueOf(v.getAsLong());
            case "float"->FloatTag.valueOf(Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16)));
            case "double"->DoubleTag.valueOf(Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16)));
            case "string"->StringTag.valueOf(v.getAsString());
            case "list"->{var list=new ListTag();for(var item:v.getAsJsonArray())list.add(tag(item));yield list;}
            case "compound"->{var map=new CompoundTag();for(var item:v.getAsJsonObject().entrySet())map.put(item.getKey(),tag(item.getValue()));yield map;}
            case "long_array"->{var array=v.getAsJsonArray();long[] words=new long[array.size()];for(int i=0;i<words.length;i++)words[i]=array.get(i).getAsLong();yield new LongArrayTag(words);}
            case "int_array"->{var array=v.getAsJsonArray();int[] words=new int[array.size()];for(int i=0;i<words.length;i++)words[i]=array.get(i).getAsInt();yield new IntArrayTag(words);}
            case "byte_array"->{var array=v.getAsJsonArray();byte[] words=new byte[array.size()];for(int i=0;i<words.length;i++)words[i]=array.get(i).getAsByte();yield new ByteArrayTag(words);}
            default->throw new AssertionError(kind);
        };
    }
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        try(var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))){
            int index=0;
            for(var line:Files.readAllLines(Path.of(args[0]))){
                out.println("case "+index++);
                var result=BelowZeroRetrogen.CODEC.parse(NbtOps.INSTANCE,tag(JsonParser.parseString(line))).result();
                if(result.isEmpty()){out.println("decoded error");continue;}
                var data=result.get();var encoded=(CompoundTag)BelowZeroRetrogen.CODEC.encodeStart(NbtOps.INSTANCE,data).getOrThrow();
                out.println("decoded "+data.targetStatus().getName()+" "+data.hasBedrockHoles()+" "+encoded.size());
                out.println("encoded_target "+((StringTag)encoded.get("target_status")).value());
                var words=(LongArrayTag)encoded.get("missing_bedrock");
                var text=new StringJoiner(",");if(words!=null)for(long word:words.getAsLongArray())text.add(Long.toUnsignedString(word,16));
                out.println("mask "+(words==null?"absent":text));
                var holes=new StringBuilder();for(int z=-17;z<=17;z++)for(int x=-17;x<=17;x++)holes.append(data.hasBedrockHole(x,z)?'1':'0');
                for(int z:new int[]{Integer.MIN_VALUE,Integer.MAX_VALUE})for(int x:new int[]{Integer.MIN_VALUE,Integer.MAX_VALUE})holes.append(data.hasBedrockHole(x,z)?'1':'0');
                out.println("holes "+holes);
            }
        }
    }
}
