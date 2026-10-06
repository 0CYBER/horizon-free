package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.serialization.JsonOps;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import it.unimi.dsi.fastutil.ints.Int2ObjectOpenHashMap;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.resources.metadata.animation.*;
import net.minecraft.resources.Identifier;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.map;

/** Invokes the pinned metadata codec and real animation tick state, without GPU draws. */
public final class VanillaSpriteAnimationOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static Object field(Object target, String name) throws Exception {
        var field=target.getClass().getDeclaredField(name); field.setAccessible(true); return field.get(target);
    }
    private static Object probe(JsonObject row) throws Exception {
        AnimationMetadataSection metadata;
        try { metadata=AnimationMetadataSection.CODEC.parse(JsonOps.INSTANCE,JsonParser.parseString(row.get("json").getAsString())).getOrThrow(); }
        catch(RuntimeException error) { return null; }
        int width=row.get("width").getAsInt(),height=row.get("height").getAsInt();
        var size=metadata.calculateFrameSize(width,height);
        Object frames=metadata.frames().map(list->list.stream().map(f->map("index",f.index(),"time",f.time().orElse(null))).toList()).orElse(null);
        var result=map("metadata",map("frames",frames,"width",metadata.frameWidth().orElse(null),"height",metadata.frameHeight().orElse(null),"frametime",metadata.defaultFrameTime(),"interpolate",metadata.interpolatedFrames()),"size",new int[]{size.width(),size.height()});
        // This guard scopes allocation to valid sprite grids. Frame-size calculation
        // above remains tested for impossible dimensions; loader errors are separate.
        if(size.width()>width || size.height()>height || width%size.width()!=0 || height%size.height()!=0) return result;
        try(var contents=new SpriteContents(Identifier.parse("horizon:probe"),size,new NativeImage(width,height,true),Optional.of(metadata),List.of(),Optional.empty())) {
            var animation=field(contents,"animatedTexture");
            if(animation==null) { result.put("animation",null); return result; }
            var nativeFrames=(List<?>)field(animation,"frames"); var sequence=new ArrayList<int[]>();
            for(var frame:nativeFrames) sequence.add(new int[]{(int)field(frame,"index"),(int)field(frame,"time")});
            var type=Class.forName("net.minecraft.client.renderer.texture.SpriteContents$AnimationState");
            var constructor=type.getDeclaredConstructors()[0];constructor.setAccessible(true);
            var state=(SpriteContents.AnimationState)constructor.newInstance(contents,animation,new Int2ObjectOpenHashMap<>(),new GpuBufferSlice[0]);
            var samples=new ArrayList<Object>();
            int ticks=row.get("ticks").getAsInt();
            for(int tick=0;tick<=ticks;tick++) {
                samples.add(sample(state,sequence));
                state.tick();
            }
            // Probe native tick transitions near very long frame durations too.
            var edges=new ArrayList<Object>();
            var frameField=type.getDeclaredField("frame");frameField.setAccessible(true);
            var subField=type.getDeclaredField("subFrame");subField.setAccessible(true);
            for(int index=0;index<sequence.size();index++) {
                int duration=sequence.get(index)[1];
                for(int sub: new int[]{0,1,duration/2,duration-2,duration-1}) {
                    if(sub<0 || sub>=duration)continue;
                    frameField.setInt(state,index);subField.setInt(state,sub);state.tick();
                    edges.add(sample(state,sequence));
                }
            }
            result.put("animation",map("sequence",sequence,"columns",field(animation,"frameRowSize"),"samples",samples,"edges",edges));
        }
        return result;
    }
    private static Object sample(SpriteContents.AnimationState state,List<int[]> sequence) throws Exception {
        int index=(int)field(state,"frame"),sub=(int)field(state,"subFrame");var current=sequence.get(index);
        int blend=(int)((float)sub/(float)current[1]*1000.0f);
        return Arrays.asList(index,sub,current[0],sequence.get((index+1)%sequence.size())[0],field(state,"isDirty"),state.needsToDraw(),blend);
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        try(var input=Files.newBufferedReader(Path.of(args[0]));var output=Files.newBufferedWriter(Path.of(args[1]))) {
            for(String line;(line=input.readLine())!=null;) {
                var row=JsonParser.parseString(line).getAsJsonObject();Object value;
                var probe=probe(row);
                value=probe==null ? map("name",row.get("name").getAsString(),"error",true) : map("name",row.get("name").getAsString(),"value",probe);
                output.write(JSON.toJson(value));output.newLine();
            }
        }
    }
}
