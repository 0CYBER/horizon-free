package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.vertex.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.client.renderer.block.dispatch.*;
import net.minecraft.client.renderer.feature.BlockModelFeatureRenderer;
import net.minecraft.client.renderer.texture.*;
import net.minecraft.client.renderer.texture.atlas.*;
import net.minecraft.client.resources.model.sprite.MaterialBaker;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.Identifier;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.RenderShape;

/** Real model selection, offset, BlockModelFeatureRenderer and sheeted decal projection. */
public final class VanillaBreakingModelOracle {
    private static final Gson JSON=new Gson();
    private static final class Recorder implements VertexConsumer {
        final List<List<float[]>> quads=new ArrayList<>();
        final List<float[]> pending=new ArrayList<>();
        float[] vertex;
        public VertexConsumer addVertex(float x,float y,float z) { vertex=new float[]{x,y,z,0,0};return this; }
        public VertexConsumer setColor(int color) {
            if (color!=-1) throw new AssertionError("decal is tinted");return this;
        }
        public VertexConsumer setColor(int r,int g,int b,int a) {
            if (r!=255||g!=255||b!=255||a!=255) throw new AssertionError("decal is tinted");return this;
        }
        public VertexConsumer setUv(float u,float v) {
            vertex[3]=u;vertex[4]=v;pending.add(vertex);
            if (pending.size()==4) { quads.add(new ArrayList<>(pending));pending.clear(); }return this;
        }
        public VertexConsumer setUv3(float u,float v) { return this; }
        public VertexConsumer setUv1(int u,int v) { return this; }
        public VertexConsumer setUv2(int u,int v) {
            if (u!=240||v!=240) throw new AssertionError("decal is not full bright");return this;
        }
        public VertexConsumer setNormal(float x,float y,float z) { return this; }
        public VertexConsumer setLineWidth(float width) { return this; }
    }
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();Bootstrap.bootStrap();
        var rows=new ArrayList<JsonObject>();var blocks=new HashSet<Block>();
        for (String line:Files.readAllLines(Path.of(args[0]))) {
            var row=JsonParser.parseString(line).getAsJsonObject();rows.add(row);
            blocks.add(Block.stateById(row.get("state").getAsInt()).getBlock());
        }
        // Reuse the existing oracle model baker rather than duplicating it.
        String helper=VanillaSectionMeshOracle.class.getName();
        var missingContents=new SpriteContents(Identifier.parse("minecraft:missingno"),new FrameSize(16,16),new NativeImage(16,16,true));
        var spriteCtor=Class.forName(helper+"$Sprite").getDeclaredConstructor(SpriteContents.class,int.class,int.class);spriteCtor.setAccessible(true);
        var missing=(TextureAtlasSprite)spriteCtor.newInstance(missingContents,1,0);
        var atlas=new SpriteLoader.Preparations(16,16,0,missing,Map.of(),CompletableFuture.completedFuture(null));
        var materialsCtor=Class.forName(helper+"$Materials").getDeclaredConstructor(SpriteLoader.Preparations.class);materialsCtor.setAccessible(true);
        var materials=(MaterialBaker)materialsCtor.newInstance(atlas);
        var modelsClass=Class.forName(helper+"$Models");
        var modelsCtor=modelsClass.getDeclaredConstructor(Set.class,MaterialBaker.class);modelsCtor.setAccessible(true);
        Object models=modelsCtor.newInstance(blocks,materials);
        var getModel=modelsClass.getDeclaredMethod("get",net.minecraft.world.level.block.state.BlockState.class);getModel.setAccessible(true);
        var put=BlockModelFeatureRenderer.class.getDeclaredMethod("putPartQuads",BlockStateModelPart.class,PoseStack.Pose.class,QuadInstance.class,int.class,int[].class,VertexConsumer.class);put.setAccessible(true);
        try (var output=Files.newBufferedWriter(Path.of(args[1]))) {
            for (var row:rows) {
                var state=Block.stateById(row.get("state").getAsInt());var recorder=new Recorder();boolean translucent=false;
                if (state.getRenderShape()==RenderShape.MODEL) {
                    var p=row.getAsJsonArray("pos");var camera=row.getAsJsonArray("camera");
                    var pos=new BlockPos(p.get(0).getAsInt(),p.get(1).getAsInt(),p.get(2).getAsInt());
                    var pose=new PoseStack();
                    pose.translate(pos.getX()-camera.get(0).getAsDouble(),pos.getY()-camera.get(1).getAsDouble(),pos.getZ()-camera.get(2).getAsDouble());
                    pose.translate(state.getOffset(pos));
                    var model=(BlockStateModel)getModel.invoke(models,state);translucent=model.hasMaterialFlag(1);
                    var parts=new ArrayList<BlockStateModelPart>();model.collectParts(RandomSource.create(state.getSeed(pos)),parts);
                    var instance=new QuadInstance();instance.setLightCoords(15728880);instance.setOverlayCoords(655360);
                    var buffer=new SheetedDecalTextureGenerator(recorder,pose.last(),1.0F);
                    for (var part:parts) put.invoke(null,part,pose.last(),instance,0,new int[0],buffer);
                }
                if (!recorder.pending.isEmpty()) throw new AssertionError("partial quad");
                output.write(JSON.toJson(Map.of("case",row.get("case").getAsInt(),"translucent",translucent,"quads",recorder.quads)));output.newLine();
            }
        }
    }
}
