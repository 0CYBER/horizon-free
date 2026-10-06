package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.math.Quadrant;
import com.mojang.serialization.JsonOps;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.renderer.block.dispatch.*;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.geometry.*;
import net.minecraft.client.resources.model.sprite.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.StrictJsonParser;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import org.joml.Matrix4fc;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.map;

/** Real registry binding, root baking and model collection for the Rust scene.
 * Empty tagged geometry isolates dispatch; FaceBakery has its separate gate. */
public final class VanillaGallerySelectionOracle {
    private static final Gson JSON=new GsonBuilder().serializeNulls().create();
    private static Object matrix(Matrix4fc value) {
        var rows=new long[3][4];
        for(int r=0;r<3;r++)for(int c=0;c<4;c++)rows[r][c]=Integer.toUnsignedLong(Float.floatToIntBits(value.get(c,r)));
        return rows;
    }
    private static JsonElement tag(String name,ModelState state) {
        var faces=new ArrayList<Object>();for(var d:Direction.values())faces.add(matrix(state.inverseFaceTransformation(d)));
        return JSON.toJsonTree(map("model",name,"matrix",matrix(state.transformation().getMatrix()),"inverse_uv",faces));
    }
    private static final class Baker implements ModelBaker {
        final IdentityHashMap<QuadCollection,JsonElement> tags=new IdentityHashMap<>();
        public ResolvedModel getModel(Identifier id) {
            return new ResolvedModel() {
                public String debugName(){return id.toString();}
                public UnbakedModel wrapped(){throw new AssertionError("unexpected wrapped model access");}
                public ResolvedModel parent(){return null;}
                public TextureSlots getTopTextureSlots(){return null;}
                public boolean getTopAmbientOcclusion(){return true;}
                public Material.Baked resolveParticleMaterial(TextureSlots slots,ModelBaker baker){return null;}
                public QuadCollection bakeTopGeometry(TextureSlots slots,ModelBaker baker,ModelState state) {
                    try {
                        var constructor=QuadCollection.class.getDeclaredConstructors()[0];constructor.setAccessible(true);
                        var empty=List.of();var result=(QuadCollection)constructor.newInstance(empty,empty,empty,empty,empty,empty,empty,empty);
                        tags.put(result,tag(id.toString(),state));return result;
                    } catch(ReflectiveOperationException error){throw new AssertionError(error);}
                }
            };
        }
        public BlockStateModelPart missingBlockModelPart(){throw new AssertionError("unexpected missing model");}
        public MaterialBaker materials(){throw new AssertionError("unexpected material bake");}
        public ModelBaker.Interner interner(){throw new AssertionError("unexpected interner");}
        public <T> T compute(ModelBaker.SharedOperationKey<T> key){return key.compute(this);}
    }
    private static <T extends Comparable<T>> BlockState set(BlockState state,Property<T> property,String value) {
        return state.setValue(property,property.getValue(value).orElseThrow());
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var report=JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        var results=new ArrayList<Object>();int index=0;
        for(var item:report.getAsJsonArray("model_gallery")) {
            var row=item.getAsJsonObject();var id=Identifier.parse(row.get("block").getAsString());
            var block=BuiltInRegistries.BLOCK.getValue(id);var state=block.defaultBlockState();
            for(var entry:row.getAsJsonObject("properties").entrySet())
                state=set(state,Objects.requireNonNull(block.getStateDefinition().getProperty(entry.getKey())),entry.getValue().getAsString());
            if(Block.getId(state)!=row.get("state_id").getAsInt())throw new AssertionError("state ID mismatch");
            String resource="/assets/"+id.getNamespace()+"/blockstates/"+id.getPath()+".json";
            String raw;try(var input=VanillaGallerySelectionOracle.class.getResourceAsStream(resource)) {
                raw=new String(Objects.requireNonNull(input).readAllBytes(),java.nio.charset.StandardCharsets.UTF_8);
            }
            var definition=BlockStateModelDispatcher.CODEC.parse(JsonOps.INSTANCE,StrictJsonParser.parse(raw)).getOrThrow();
            var root=definition.instantiate(block.getStateDefinition(),id::toString).get(state);
            var baker=new Baker();var model=Objects.requireNonNull(root).bake(state,baker);
            var pos=row.getAsJsonArray("position");var position=new BlockPos(pos.get(0).getAsInt(),pos.get(1).getAsInt(),pos.get(2).getAsInt());
            var parts=new ArrayList<BlockStateModelPart>();model.collectParts(new LegacyRandomSource(state.getSeed(position)),parts);
            var actual=new ArrayList<JsonElement>();
            for(var part:parts)actual.add(Objects.requireNonNull(baker.tags.get(((SimpleModelWrapper)part).quads())));
            var expected=new ArrayList<JsonElement>();
            for(var selected:row.getAsJsonArray("models")) {
                var v=selected.getAsJsonObject();var t=v.getAsJsonArray("rotations");
                var transform=new Variant.SimpleModelState(Quadrant.values()[t.get(0).getAsInt()],Quadrant.values()[t.get(1).getAsInt()],Quadrant.values()[t.get(2).getAsInt()],v.get("uv_lock").getAsBoolean()).asModelState();
                expected.add(tag(v.get("model").getAsString(),transform));
            }
            if(!actual.equals(expected))throw new AssertionError("gallery selection mismatch at "+index+" "+id+": native="+actual+" Rust="+expected);
            results.add(map("case",index++,"state_id",Block.getId(state),"models",actual));
        }
        Files.writeString(Path.of(args[1]),JSON.toJson(results));
    }
}
