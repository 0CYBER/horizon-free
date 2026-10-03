package dev.horizonlab.mixin;

import com.seibel.distanthorizons.core.render.QuadTree.QuadTreeTickNodeHolder;
import it.unimi.dsi.fastutil.objects.ObjectOpenHashSet;
import java.util.HashSet;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Replace only the holder's private membership test; published action sets keep their order. */
@Mixin(value = QuadTreeTickNodeHolder.class, remap = false)
public abstract class DhTickDedupMixin {
    @Unique private final ObjectOpenHashSet<Object> horizon$presentNodes = new ObjectOpenHashSet<>();

    @Redirect(method = {"addEnableNode", "addDisableNode", "addEnableDeleteChildrenNode", "removeEnableAndDisableNode"},
        at = @At(value = "INVOKE", target = "Ljava/util/HashSet;add(Ljava/lang/Object;)Z", ordinal = 0))
    private boolean horizon$recordPresentNode(HashSet<?> original, Object node) {
        // In pinned DH 3.3.4 each method's first add targets presentNodes.
        // The original branches and action collections remain responsible for every transition.
        return horizon$presentNodes.add(node);
    }

    @Inject(method = "clear", at = @At("HEAD"))
    private void horizon$clearPresentNodes(CallbackInfo callback) {
        horizon$presentNodes.clear(); // Release references at the same boundary; retain reusable table capacity.
    }
}
