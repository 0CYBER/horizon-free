package dev.horizonlab.parity;
import com.google.gson.*;import java.io.*;import java.lang.reflect.*;import java.nio.file.*;
import net.minecraft.nbt.*;import net.minecraft.world.food.FoodData;import net.minecraft.world.level.storage.*;import net.minecraft.util.ProblemReporter;
/** Native TagValueInput/Output hunger persistence and numeric coercions. */
public final class VanillaFoodStorageOracle {
    static final String[] KEYS={"foodLevel","foodTickTimer","foodSaturationLevel","foodExhaustionLevel"};
    static String bits(float v){return String.format("%08x",Float.floatToIntBits(v));}
    static float single(JsonElement v){return Float.intBitsToFloat(Integer.parseUnsignedInt(v.getAsString(),16));}
    static Tag tag(JsonObject o){int type=o.get("type").getAsInt();var v=o.get("value");return switch(type){
        case 1->ByteTag.valueOf(v.getAsByte());case 2->ShortTag.valueOf(v.getAsShort());case 3->IntTag.valueOf(v.getAsInt());case 4->LongTag.valueOf(Long.parseLong(v.getAsString()));
        case 5->FloatTag.valueOf(single(v));case 6->DoubleTag.valueOf(Double.longBitsToDouble(Long.parseUnsignedLong(v.getAsString(),16)));
        case 7->new ByteArrayTag(new byte[]{1});case 8->StringTag.valueOf("17");case 9->new ListTag();case 10->new CompoundTag();case 11->new IntArrayTag(new int[]{2});case 12->new LongArrayTag(new long[]{3});default->throw new AssertionError();};}
    static Field field(String n)throws Exception{var f=FoodData.class.getDeclaredField(n);f.setAccessible(true);return f;}
    public static void main(String[] args)throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();Path dir=Path.of(args[1]);var exhaustion=field("exhaustionLevel");var timer=field("tickTimer");
        try(var out=new PrintWriter(Files.newBufferedWriter(dir.resolve(args[0]+".txt")));var lines=Files.lines(dir.resolve("inputs.txt"))){for(var line:(Iterable<String>)lines::iterator){var f=JsonParser.parseString(line).getAsJsonObject();var food=new FoodData();food.setFoodLevel(-9);food.setSaturation(-7);timer.setInt(food,83);exhaustion.setFloat(food,99);
            if(f.get("mode").getAsString().equals("read")){var compound=new CompoundTag();var tags=f.getAsJsonArray("tags");for(int i=0;i<4;i++){var t=tags.get(i).getAsJsonObject();if(t.get("type").getAsInt()!=0)compound.put(KEYS[i],tag(t));}food.readAdditionalSaveData(TagValueInput.create(ProblemReporter.DISCARDING,net.minecraft.core.HolderLookup.Provider.create(java.util.stream.Stream.empty()),compound));}
            else {food.setFoodLevel(f.get("food").getAsInt());food.setSaturation(single(f.get("saturation")));timer.setInt(food,f.get("timer").getAsInt());exhaustion.setFloat(food,single(f.get("exhaustion")));}
            out.println("state "+food.getFoodLevel()+" "+timer.getInt(food)+" "+bits(food.getSaturationLevel())+" "+bits(exhaustion.getFloat(food)));
            var output=TagValueOutput.createWithoutContext(ProblemReporter.DISCARDING);output.putInt("unrelated",42);output.putString("foodLevel","replace");food.addAdditionalSaveData(output);var saved=output.buildResult();out.println("saved "+saved.size()+" "+saved.getIntOr("unrelated",-1));for(var key:KEYS){var t=(NumericTag)saved.get(key);out.println("tag "+t.getId()+" "+(t instanceof IntTag?Integer.toString(t.intValue()):bits(t.floatValue())));}
            food.readAdditionalSaveData(TagValueInput.create(ProblemReporter.DISCARDING,net.minecraft.core.HolderLookup.Provider.create(java.util.stream.Stream.empty()),saved));out.println("roundtrip "+food.getFoodLevel()+" "+timer.getInt(food)+" "+bits(food.getSaturationLevel())+" "+bits(exhaustion.getFloat(food)));
        }}
    }
}
