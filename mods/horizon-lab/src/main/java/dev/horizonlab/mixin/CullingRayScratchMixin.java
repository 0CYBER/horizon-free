package dev.horizonlab.mixin;

import com.logisticscraft.occlusionculling.util.Vec3d;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** Keep temporary numeric vectors on the already mutable, single-caller culling instance. */
@Mixin(targets = "com.logisticscraft.occlusionculling.OcclusionCullingInstance", remap = false)
public abstract class CullingRayScratchMixin {
    @Unique private final Vec3d horizon$rayDirection = new Vec3d(0, 0, 0);
    @Unique private final Vec3d horizon$inverseDirection = new Vec3d(0, 0, 0);

    @Redirect(method = "isVisible", at = @At(value = "NEW",
        target = "(DDD)Lcom/logisticscraft/occlusionculling/util/Vec3d;"))
    private Vec3d horizon$direction(double x, double y, double z) {
        horizon$rayDirection.set(x, y, z);
        return horizon$rayDirection;
    }

    @Redirect(method = "rayIntersection", at = @At(value = "NEW",
        target = "(DDD)Lcom/logisticscraft/occlusionculling/util/Vec3d;"))
    private Vec3d horizon$inverse(double x, double y, double z) {
        horizon$inverseDirection.set(x, y, z);
        return horizon$inverseDirection;
    }
}
