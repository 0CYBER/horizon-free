package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.math.Quadrant;
import com.mojang.serialization.JsonOps;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.renderer.block.dispatch.*;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.geometry.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.sprite.*;
import net.minecraft.core.*;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.StrictJsonParser;
import net.minecraft.world.level.block.*;
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
    private record PartTag(Identifier model,ModelState transform,JsonElement signature) {}
    private static final class Baker implements ModelBaker {
        final IdentityHashMap<QuadCollection,PartTag> tags=new IdentityHashMap<>();
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
                        tags.put(result,new PartTag(id,state,tag(id.toString(),state)));return result;
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
    private static Object scene(JsonObject report)throws Exception {
        int chunks=report.get("resident_chunks").getAsInt();int diameter=(int)Math.sqrt(chunks);
        if(diameter*diameter!=chunks || diameter%2==0)throw new AssertionError("unsupported scene grid");
        int radius=diameter/2;var world=new HashMap<BlockPos,BlockState>();
        String[] layers={"bedrock","dirt","stone","sand","oak_planks","bricks"};
        var materials=new ArrayList<BlockState>();for(var name:layers)materials.add(BuiltInRegistries.BLOCK.getValue(Identifier.parse(name)).defaultBlockState());
        for(int cz=-radius;cz<=radius;cz++)for(int cx=-radius;cx<=radius;cx++)
            for(int z=0;z<16;z++)for(int x=0;x<16;x++)for(int y=0;y<4;y++) {
                int layer=y==0?0:y<3?1:((cx+cz)&1)==0?2:3;
                world.put(new BlockPos(cx*16+x,y,cz*16+z),materials.get(layer));
            }
        for(int y=4;y<11;y++)for(int z=-10;z<=10;z++)for(int x=-10;x<=10;x++)
            if(Math.max(Math.abs(x),Math.abs(z))<=14-y)world.put(new BlockPos(x,y,z),materials.get((y&1)==0?4:5));
        for(var item:report.getAsJsonArray("model_gallery")) {
            var row=item.getAsJsonObject();var p=row.getAsJsonArray("position");
            world.put(new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt()),Block.stateById(row.get("state_id").getAsInt()));
        }
        var roots=new HashMap<BlockState,BlockStateModel.UnbakedRoot>();
        for(var state:new HashSet<>(world.values())) {
            var id=BuiltInRegistries.BLOCK.getKey(state.getBlock());
            try(var input=VanillaGallerySelectionOracle.class.getResourceAsStream("/assets/"+id.getNamespace()+"/blockstates/"+id.getPath()+".json")) {
                var raw=new String(Objects.requireNonNull(input).readAllBytes(),java.nio.charset.StandardCharsets.UTF_8);
                roots.putAll(BlockStateModelDispatcher.CODEC.parse(JsonOps.INSTANCE,StrictJsonParser.parse(raw)).getOrThrow().instantiate(state.getBlock().getStateDefinition(),id::toString));
            }
        }
        var baker=new Baker();var models=new HashMap<BlockState,BlockStateModel>();
        for(var state:new HashSet<>(world.values()))models.put(state,Objects.requireNonNull(roots.get(state)).bake(state,baker));
        var unbaked=new HashMap<Identifier,UnbakedModel>();var pending=new ArrayList<Identifier>();
        for(var part:baker.tags.values())pending.add(part.model());
        for(int i=0;i<pending.size();i++) {
            var id=pending.get(i);if(unbaked.containsKey(id))continue;
            try(var input=VanillaGallerySelectionOracle.class.getResourceAsStream("/assets/"+id.getNamespace()+"/models/"+id.getPath()+".json")) {
                var model=CuboidModel.fromStream(new java.io.InputStreamReader(Objects.requireNonNull(input),java.nio.charset.StandardCharsets.UTF_8));
                unbaked.put(id,model);if(model.parent()!=null)pending.add(model.parent());
            }
        }
        var discovery=new ModelDiscovery(unbaked,MissingCuboidModel.missingModel());
        discovery.addRoot(resolver->{for(var id:unbaked.keySet())resolver.markDependency(id);});var resolved=discovery.resolve();
        int visible=0,culled=0;LeavesBlock.setCutoutLeaves(true);
        var parts=new ArrayList<BlockStateModelPart>();
        for(var entry:world.entrySet()) {
            var pos=entry.getKey();var state=entry.getValue();parts.clear();
            models.get(state).collectParts(new LegacyRandomSource(state.getSeed(pos)),parts);
            for(var selected:parts) {
                var part=baker.tags.get(((SimpleModelWrapper)selected).quads());
                var geometry=resolved.get(part.model()).getTopGeometry();if(geometry==UnbakedGeometry.EMPTY)continue;
                for(var element:((UnbakedCuboidGeometry)geometry).elements())for(var face:element.faces().values()) {
                    var direction=face.cullForDirection();
                    if(direction!=null)direction=Direction.rotate(part.transform().transformation().getMatrix(),direction);
                    if(direction==null || Block.shouldRenderFace(state,world.getOrDefault(pos.relative(direction),Blocks.AIR.defaultBlockState()),direction))visible++;else culled++;
                }
            }
        }
        if(visible!=report.get("visible_model_faces").getAsInt() || culled!=report.get("culled_model_faces").getAsInt() || world.size()!=report.get("model_blocks").getAsInt())
            throw new AssertionError("native chunk model totals disagree: "+world.size()+" blocks / "+visible+" visible / "+culled+" culled");
        return map("blocks",world.size(),"visible_faces",visible,"culled_faces",culled);
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();net.minecraft.server.Bootstrap.bootStrap();
        var report=JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonObject();
        VanillaHeightmapOracle.bindTags(JsonParser.parseString(Files.readString(Path.of(args[2]).resolve("block-tags.json"))).getAsJsonObject());
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
            for(var part:parts)actual.add(Objects.requireNonNull(baker.tags.get(((SimpleModelWrapper)part).quads())).signature());
            var expected=new ArrayList<JsonElement>();
            for(var selected:row.getAsJsonArray("models")) {
                var v=selected.getAsJsonObject();var t=v.getAsJsonArray("rotations");
                var transform=new Variant.SimpleModelState(Quadrant.values()[t.get(0).getAsInt()],Quadrant.values()[t.get(1).getAsInt()],Quadrant.values()[t.get(2).getAsInt()],v.get("uv_lock").getAsBoolean()).asModelState();
                expected.add(tag(v.get("model").getAsString(),transform));
            }
            if(!actual.equals(expected))throw new AssertionError("gallery selection mismatch at "+index+" "+id+": native="+actual+" Rust="+expected);
            results.add(map("case",index++,"state_id",Block.getId(state),"models",actual));
        }
        Files.writeString(Path.of(args[1]),JSON.toJson(map("gallery",results,"scene",scene(report))));
    }
}
