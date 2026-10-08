package dev.horizonlab.mixin;

import com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.BufferMergeDirectionEnum;
import com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.BufferQuad;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** Reuse one discarded successful-premerge quad only when the existing pool needs a new object. */
@Mixin(targets = "com.seibel.distanthorizons.core.dataObjects.render.bufferBuilding.LodQuadBuilder", remap = false)
public abstract class DhQuadReuseMixin {
    @Unique private BufferQuad horizon$discardedQuad;

    @Redirect(method = "getOrCreateBufferQuad", at = @At(value = "NEW",
        target = "com/seibel/distanthorizons/core/dataObjects/render/bufferBuilding/BufferQuad"), require = 2, allow = 2)
    private BufferQuad horizon$freshQuad() {
        BufferQuad available = horizon$discardedQuad;
        horizon$discardedQuad = null;
        return available == null ? new BufferQuad() : available;
    }

    @Redirect(method = "addQuadAdj", at = @At(value = "INVOKE",
        target = "Lcom/seibel/distanthorizons/core/dataObjects/render/bufferBuilding/BufferQuad;tryMerge(Lcom/seibel/distanthorizons/core/dataObjects/render/bufferBuilding/BufferQuad;Lcom/seibel/distanthorizons/core/dataObjects/render/bufferBuilding/BufferMergeDirectionEnum;)Z"),
        require = 2, allow = 2)
    private boolean horizon$merge(BufferQuad receiver, BufferQuad incoming, BufferMergeDirectionEnum direction) {
        boolean merged = receiver.tryMerge(incoming, direction);
        if (merged && horizon$discardedQuad == null) horizon$discardedQuad = incoming;
        return merged;
    }
}
