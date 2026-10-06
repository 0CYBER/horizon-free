package dev.horizonlab.parity;

import com.google.gson.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.renderer.block.dispatch.*;
import net.minecraft.client.resources.model.sprite.Material;
import net.minecraft.util.RandomSource;
import net.minecraft.util.random.*;
import net.minecraft.world.level.levelgen.*;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.map;

/** Real weighted and multipart collectParts, with observable inert leaf models. */
public final class VanillaModelChoiceOracle {
    private static final Gson JSON=new GsonBuilder().serializeNulls().create();
    private record Leaf(String tag,List<String> output) implements BlockStateModel {
        public Material.Baked particleMaterial(){return null;}
        public int materialFlags(){return 0;}
        public void collectParts(RandomSource random,List<BlockStateModelPart> parts){output.add(tag);}
    }
    private static BlockStateModel choice(JsonElement value,int index,List<String> output) {
        if(value.isJsonNull())return new Leaf("horizon:part"+index+"_single",output);
        var list=new ArrayList<Weighted<BlockStateModel>>();int variant=0;
        for(var weight:value.getAsJsonArray())list.add(new Weighted<>(new Leaf("horizon:part"+index+"_variant"+variant++,output),weight.getAsInt()));
        return new WeightedVariants(WeightedList.of(list));
    }
    private static Object probe(JsonObject row) throws Exception {
        var emitted=new ArrayList<String>();var values=row.getAsJsonArray("parts");
        BlockStateModel model;
        if(!row.get("multipart").getAsBoolean())model=choice(values.get(0),0,emitted);
        else {
            var selectors=new ArrayList<Object>();var selected=new HashSet<Integer>();
            for(var i:row.getAsJsonArray("selected"))selected.add(i.getAsInt());
            var selectorType=Class.forName("net.minecraft.client.renderer.block.dispatch.multipart.MultiPartModel$Selector");
            var selectorConstructor=selectorType.getDeclaredConstructors()[0];selectorConstructor.setAccessible(true);
            for(int i=0;i<values.size();i++) {
                boolean enabled=selected.contains(i);
                java.util.function.Predicate<net.minecraft.world.level.block.state.BlockState> predicate=state->enabled;
                selectors.add(selectorConstructor.newInstance(predicate,choice(values.get(i),i,emitted)));
            }
            var sharedType=Class.forName("net.minecraft.client.renderer.block.dispatch.multipart.MultiPartModel$SharedBakedState");
            var sharedConstructor=sharedType.getDeclaredConstructors()[0];sharedConstructor.setAccessible(true);
            var shared=sharedConstructor.newInstance(selectors);
            var constructor=Class.forName("net.minecraft.client.renderer.block.dispatch.multipart.MultiPartModel").getDeclaredConstructors()[0];constructor.setAccessible(true);
            model=(BlockStateModel)constructor.newInstance(shared,null);
        }
        long seed=row.get("seed").getAsLong();
        RandomSource random=row.get("random").getAsString().equals("legacy")?new LegacyRandomSource(seed):new XoroshiroRandomSource(seed);
        var traces=new ArrayList<Object>();var parts=new ArrayList<BlockStateModelPart>();
        for(int i=0;i<row.get("repeats").getAsInt();i++) {
            emitted.clear();model.collectParts(random,parts);
            traces.add(map("models",new ArrayList<>(emitted),"next_long",random.nextLong(),"next_int",random.nextInt()));
        }
        return traces;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        try(var input=Files.newBufferedReader(Path.of(args[0]));var output=Files.newBufferedWriter(Path.of(args[1]))) {
            for(String line;(line=input.readLine())!=null;) {
                var row=JsonParser.parseString(line).getAsJsonObject();
                output.write(JSON.toJson(map("name",row.get("name").getAsString(),"trace",probe(row))));output.newLine();
            }
        }
    }
}
