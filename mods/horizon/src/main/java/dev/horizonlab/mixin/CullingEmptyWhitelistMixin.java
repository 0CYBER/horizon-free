package dev.horizonlab.mixin;

import java.util.Collections;
import java.util.Iterator;
import java.util.Set;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/** Empty dynamic whitelists have no handlers to visit; nonempty sets retain their own iterator. */
@Mixin(targets = "dev.tr7zw.entityculling.EntityCullingModBase", remap = false)
public abstract class CullingEmptyWhitelistMixin {
    @Redirect(method = {"isEntityDynamicWhitelisted", "isBlockEntityDynamicWhitelisted"},
        at = @At(value = "INVOKE", target = "Ljava/util/Set;iterator()Ljava/util/Iterator;"))
    private Iterator<?> horizon$iterateDynamicHandlers(Set<?> handlers) {
        // Check the live set on every call. Registration, order, callbacks and nonempty iteration stay upstream.
        return handlers.isEmpty() ? Collections.emptyIterator() : handlers.iterator();
    }
}
