package dev.horizonlab.dispatch.fixture;

import com.logisticscraft.occlusionculling.OcclusionCullingInstance;
import com.logisticscraft.occlusionculling.util.Vec3d;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;
import org.spongepowered.asm.mixin.gen.Invoker;

@Mixin(value = OcclusionCullingInstance.class, remap = false)
public interface CullingRayAccess {
    @Invoker("rayIntersection") boolean horizon$intersects(int[] block, Vec3d camera, Vec3d direction);
    @Invoker("isVisible") boolean horizon$visible(Vec3d camera, Vec3d[] points, int count);
    @Accessor("allowRayChecks") void horizon$checks(boolean value);
    @Accessor("lastHitBlock") int[] horizon$lastBlock();
    @Accessor("cameraPos") int[] horizon$cameraBlock();
}
