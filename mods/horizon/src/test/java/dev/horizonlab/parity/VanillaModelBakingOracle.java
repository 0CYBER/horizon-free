package dev.horizonlab.parity;

import com.google.gson.*;
import com.mojang.blaze3d.platform.NativeImage;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.resources.model.*;
import net.minecraft.client.resources.model.cuboid.*;
import net.minecraft.client.resources.model.geometry.*;
import net.minecraft.client.resources.model.sprite.Material;
import net.minecraft.client.renderer.block.dispatch.ModelState;
import net.minecraft.client.renderer.block.dispatch.Variant;
import com.mojang.math.Quadrant;
import net.minecraft.client.renderer.texture.*;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.core.Direction;
import net.minecraft.resources.Identifier;
import org.joml.*;
import static dev.horizonlab.parity.VanillaCuboidModelOracle.map;

/** Geometry-only native baker probe with a real 16x16 sprite in a unit atlas. */
public final class VanillaModelBakingOracle {
    private static final Gson JSON = new GsonBuilder().serializeNulls().create();
    private static final class Sprite extends TextureAtlasSprite {
        Sprite(SpriteContents contents) { super(Identifier.parse("horizon:atlas"),contents,16,16,0,0,0); }
    }
    private static final ModelBaker.Interner INTERNER = new ModelBaker.Interner() {
        public Vector3fc vector(Vector3fc vector) { return vector; }
        public BakedQuad.MaterialInfo materialInfo(BakedQuad.MaterialInfo value) { return value; }
    };
    private static long bits(float value) { return Integer.toUnsignedLong(Float.floatToIntBits(value)); }
    private static Object matrix(CuboidRotation rotation) {
        if(rotation==null)return null;
        var m=rotation.transform();
        return new long[][]{{bits(m.m00()),bits(m.m10()),bits(m.m20())},
            {bits(m.m01()),bits(m.m11()),bits(m.m21())},{bits(m.m02()),bits(m.m12()),bits(m.m22())}};
    }
    private static Map<String,Object> bake(ResolvedModel model, Sprite sprite, ModelState state) throws Exception {
        var geometry=model.getTopGeometry();
        var quads=new ArrayList<Object>(); var matrices=new ArrayList<Object>();
        if(geometry==UnbakedGeometry.EMPTY)return map("quads",quads,"matrices",matrices);
        var textures=model.getTopTextureSlots();
        var defaultUv=FaceBakery.class.getDeclaredMethod("defaultFaceUV",Vector3fc.class,Vector3fc.class,Direction.class);defaultUv.setAccessible(true);
        for(var element:((UnbakedCuboidGeometry)geometry).elements()) {
            matrices.add(matrix(element.rotation()));
            for(var entry:element.faces().entrySet()) {
                var face=entry.getValue();
                var material=textures.getMaterial(face.texture());
                if(material==null)material=new Material(Identifier.parse("minecraft:missingno"));
                var uv=face.uvs();
                if(uv==null)uv=(CuboidFace.UVs)defaultUv.invoke(null,element.from(),element.to(),entry.getKey());
                var info=new BakedQuad.MaterialInfo(sprite,null,null,null,null,face.tintIndex(),element.shadeDirectionOverride(),element.lightEmission());
                var quad=FaceBakery.bakeQuad(INTERNER,element.from(),element.to(),uv,face.rotation(),info,entry.getKey(),state,element.rotation());
                var positions=new long[4][3];var uvs=new long[4][2];
                for(int i=0;i<4;i++) {
                    var p=quad.position(i);positions[i]=new long[]{bits(p.x()),bits(p.y()),bits(p.z())};
                    long packed=quad.packedUV(i);uvs[i]=new long[]{packed >>> 32,packed & 0xffffffffL};
                }
                quads.add(map("positions",positions,"uv",uvs,"facing",quad.direction().getName(),
                    "cull",face.cullForDirection()==null?null:Direction.rotate(state.transformation().getMatrix(),face.cullForDirection()).getName(),"tint",face.tintIndex(),
                    "shade",element.shadeDirectionOverride()==null?null:element.shadeDirectionOverride().getName(),
                    "light",element.lightEmission(),"material",map("sprite",material.sprite().toString(),"translucent",material.forceTranslucent())));
            }
        }
        return map("quads",quads,"matrices",matrices);
    }
    private static Object affine(Matrix4fc matrix) {
        var rows=new long[3][4];
        for(int r=0;r<3;r++)for(int c=0;c<4;c++)rows[r][c]=bits(matrix.get(c,r));
        return rows;
    }
    public static void main(String[] args) throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        if(org.joml.Options.FASTMATH || org.joml.Options.USE_MATH_FMA)throw new AssertionError("Expected default JOML math options");
        var image=new NativeImage(16,16,true);
        var contents=new SpriteContents(Identifier.parse("horizon:probe"),new FrameSize(16,16),image);
        var sprite=new Sprite(contents);
        try (contents; var input=Files.newBufferedReader(Path.of(args[0])); var output=Files.newBufferedWriter(Path.of(args[1]))) {
            int index=0;
            for(String line;(line=input.readLine())!=null;index++) {
                var row=JsonParser.parseString(line).getAsJsonObject();var models=new HashMap<Identifier,UnbakedModel>();
                for(var entry:row.getAsJsonObject("models").entrySet())models.put(Identifier.parse(entry.getKey()),CuboidModel.fromStream(new StringReader(entry.getValue().getAsString())));
                var discovery=new ModelDiscovery(models,MissingCuboidModel.missingModel());
                discovery.addRoot(resolver->{for(var root:row.getAsJsonArray("roots"))resolver.markDependency(Identifier.parse(root.getAsString()));});
                var resolved=discovery.resolve();
                ModelState state=new ModelState(){};
                if(row.has("transform")) {
                    var t=row.getAsJsonArray("transform");
                    state=new Variant.SimpleModelState(Quadrant.values()[t.get(0).getAsInt()],Quadrant.values()[t.get(1).getAsInt()],Quadrant.values()[t.get(2).getAsInt()],t.get(3).getAsBoolean()).asModelState();
                }
                for(var root:row.getAsJsonArray("roots")) {
                    String name=root.getAsString();Map<String,Object> value;
                    try {value=bake(Objects.requireNonNull(resolved.get(Identifier.parse(name))),sprite,state);
                        if(row.has("transform")) {
                            var inverse=new ArrayList<Object>();
                            for(var d:Direction.values())inverse.add(affine(state.inverseFaceTransformation(d)));
                            value.put("transform",map("matrix",affine(state.transformation().getMatrix()),"inverse_uv",inverse));
                        }}
                    catch(RuntimeException error) {output.write(JSON.toJson(map("case",index,"name",name,"error",true)));output.newLine();continue;}
                    output.write(JSON.toJson(map("case",index,"name",name,"baked",value)));output.newLine();
                }
            }
        }
    }
}
