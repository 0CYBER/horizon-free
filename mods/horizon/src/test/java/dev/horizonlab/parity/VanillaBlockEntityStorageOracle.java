package dev.horizonlab.parity;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.nbt.*;
import net.minecraft.resources.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.*;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.storage.loot.LootTable;

/** Native proto-chunk entity storage and numeric position transport. */
public final class VanillaBlockEntityStorageOracle {
    private static final Comparator<BlockPos> ORDER=Comparator.<BlockPos>comparingInt(BlockPos::getX).thenComparingInt(BlockPos::getY).thenComparingInt(BlockPos::getZ);
    private static PalettedContainerFactory factory;
    private static String xyz(BlockPos p){return p.getX()+" "+p.getY()+" "+p.getZ();}
    private static final class Input {
        String[] words;int cursor=1;Input(String line){words=line.split(" ");}
        int integer(){return Integer.parseInt(words[cursor++]);}long number(){return Long.parseLong(words[cursor++]);}
        BlockPos pos(){return new BlockPos(integer(),integer(),integer());}
        Tag tag(){String type=words[cursor++],value=words[cursor++];return switch(type){
            case "none"->null;case "byte"->ByteTag.valueOf(Byte.parseByte(value));case "short"->ShortTag.valueOf(Short.parseShort(value));case "int"->IntTag.valueOf(Integer.parseInt(value));case "long"->LongTag.valueOf(Long.parseLong(value));
            case "float"->FloatTag.valueOf(Float.intBitsToFloat(Integer.parseUnsignedInt(value,16)));case "double"->DoubleTag.valueOf(Double.longBitsToDouble(Long.parseUnsignedLong(value,16)));case "string"->StringTag.valueOf(value);default->throw new AssertionError(type);
        };}
    }
    private static String atom(Tag tag){
        if(tag==null)return "none";
        if(tag instanceof FloatTag value)return "float:"+Integer.toUnsignedString(Float.floatToRawIntBits(value.floatValue()),16);
        if(tag instanceof DoubleTag value)return "double:"+Long.toUnsignedString(Double.doubleToRawLongBits(value.doubleValue()),16);
        if(tag instanceof NumericTag value)return tag.getId()+":"+value.longValue();
        return tag.getId()+":invalid";
    }
    private static String pending(CompoundTag tag){return tag==null?"none":tag.getLongOr("mark",0)+" "+atom(tag.get("x"))+" "+atom(tag.get("y"))+" "+atom(tag.get("z"));}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        Holder<Biome> biome=Holder.Reference.createStandAlone(new HolderOwner<Biome>(){},Biomes.PLAINS);var ids=new IdMapper<Holder<Biome>>();ids.add(biome);
        factory=new PalettedContainerFactory(Strategy.createForBlockStates(Block.BLOCK_STATE_REGISTRY),Blocks.AIR.defaultBlockState(),null,Strategy.createForBiomes(ids),biome,null);
        var registries=RegistryAccess.fromRegistryOfRegistries(BuiltInRegistries.REGISTRY);
        var loot=ResourceKey.create(Registries.LOOT_TABLE,Identifier.parse("horizon:storage_probe"));
        ProtoChunk chunk=null;
        try(var input=Files.newBufferedReader(Path.of(args[0]));var out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))){
            String line;int index=0;
            while((line=input.readLine())!=null){var in=new Input(line);String op=in.words[0];out.println("command "+index+++" "+op);
                switch(op){
                    case "reset"->chunk=new ProtoChunk(new ChunkPos(in.integer(),in.integer()),UpgradeData.EMPTY,LevelHeightAccessor.create(-64,384),factory,null);
                    case "pending","resolve"->{var tag=new CompoundTag();tag.putLong("mark",in.number());for(String axis:new String[]{"x","y","z"}){var value=in.tag();if(value!=null)tag.put(axis,value);}out.println("position "+xyz(BlockEntity.getPosFromTag(chunk.getPos(),tag)));if(op.equals("pending"))chunk.setBlockEntityNbt(tag);}
                    case "active"->{var p=in.pos();var entity=new ChestBlockEntity(p,Blocks.CHEST.defaultBlockState());entity.setLootTable(loot,in.number());chunk.setBlockEntity(entity);}
                    case "remove"->chunk.removeBlockEntity(in.pos());
                    case "mutate"->{var p=in.pos();int x=in.integer();var tag=chunk.getBlockEntityNbt(p);if(tag!=null)tag.putInt("x",x);}
                    case "get","save"->{var p=in.pos();if(op.equals("get")){var active=chunk.getBlockEntity(p);out.println("active "+(active==null?"none":((ChestBlockEntity)active).getLootTableSeed()));out.println("pending "+pending(chunk.getBlockEntityNbt(p)));}else{var tag=chunk.getBlockEntityNbtForSaving(p,registries);if(chunk.getBlockEntity(p)!=null)out.println("saved active "+tag.getStringOr("id","")+" "+tag.getIntOr("x",0)+" "+tag.getIntOr("y",0)+" "+tag.getIntOr("z",0)+" "+tag.getLongOr("LootTableSeed",0));else out.println("saved pending "+pending(tag));}}
                    case "dump"->{var positions=new TreeSet<BlockPos>(ORDER);positions.addAll(chunk.getBlockEntitiesPos());for(var p:positions){var entity=chunk.getBlockEntity(p);out.println("entry "+xyz(p)+" "+(entity==null?"none":((ChestBlockEntity)entity).getLootTableSeed())+" "+pending(chunk.getBlockEntityNbt(p)));}out.println("size "+positions.size()+" "+chunk.getBlockEntityNbts().size());}
                    default->throw new AssertionError(op);
                }
                if(in.cursor!=in.words.length)throw new AssertionError("extra input");
            }
        }
    }
}
