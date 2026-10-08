package dev.horizonlab.mixin;

import dev.horizonlab.geometry.RelativeTerrainPosition;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** Replace only the numeric conversion in the sampled full-data lookup. */
@Mixin(targets = "com.seibel.distanthorizons.core.dataObjects.fullData.sources.FullDataSourceV2", remap = false)
public abstract class DhRelativePositionMixin {
    @Redirect(method = "getDataPointAtBlockPos", at = @At(value = "INVOKE",
        target = "Lcom/seibel/distanthorizons/core/pos/DhSectionPos;getDhSectionRelativePositionForDetailLevel(JB)J"))
    private long horizon$relativePosition(long position, byte outputLevel) {
        return RelativeTerrainPosition.resolve(position, outputLevel);
    }
}
