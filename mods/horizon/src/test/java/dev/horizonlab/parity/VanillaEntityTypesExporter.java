package dev.horizonlab.parity;

import com.google.gson.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.ai.attributes.DefaultAttributes;

/** Original exporter of entity type dimensions, categories and default attributes. */
public final class VanillaEntityTypesExporter {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        var out = new JsonObject();
        for (var type : BuiltInRegistries.ENTITY_TYPE) {
            var o = new JsonObject();
            var d = type.getDimensions();
            o.addProperty("width", d.width());
            o.addProperty("height", d.height());
            o.addProperty("eye_height", d.eyeHeight());
            o.addProperty("category", type.getCategory().getName());
            o.addProperty("fire_immune", type.fireImmune());
            if (DefaultAttributes.hasSupplier(type)) {
                var supplier = DefaultAttributes.getSupplier((EntityType<? extends LivingEntity>) type);
                var attributes = new JsonObject();
                for (var attribute : BuiltInRegistries.ATTRIBUTE.asHolderIdMap()) {
                    if (supplier.hasAttribute(attribute))
                        attributes.addProperty(attribute.unwrapKey().orElseThrow().identifier().toString(), supplier.getBaseValue(attribute));
                }
                o.add("attributes", attributes);
            }
            out.add(BuiltInRegistries.ENTITY_TYPE.getKey(type).toString(), o);
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(out), StandardCharsets.UTF_8);
    }
}
