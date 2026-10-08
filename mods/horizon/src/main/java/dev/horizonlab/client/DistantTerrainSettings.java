package dev.horizonlab.client;

import com.seibel.distanthorizons.api.DhApi;
import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfig;
import com.seibel.distanthorizons.api.interfaces.config.IDhApiConfigValue;
import java.util.LinkedHashMap;
import java.util.Map;

/** Read effective API values once at capture boundaries; never change terrain settings. */
final class DistantTerrainSettings {
    private DistantTerrainSettings() {}

    static Map<String, Object> read() { return read(DhApi.Delayed.configs); }

    static Map<String, Object> read(IDhApiConfig configs) {
        if (configs == null) return Map.of("dh_capture_status", "unavailable");
        Map<String, Object> values = new LinkedHashMap<>();
        try {
            var graphics = configs.graphics();
            put(values, "dh_rendering_enabled", graphics.renderingEnabled());
            put(values, "dh_renderer_mode", graphics.renderingMode());
            put(values, "dh_lod_radius_chunks", graphics.chunkRenderDistance());
            put(values, "dh_horizontal_quality", graphics.horizontalQuality());
            put(values, "dh_vertical_quality", graphics.verticalQuality());
            put(values, "dh_max_horizontal_resolution", graphics.maxHorizontalResolution());
            put(values, "dh_transparency", graphics.transparency());
            put(values, "dh_lod_shading", graphics.lodShading());
            put(values, "dh_lod_only_mode", graphics.lodOnlyMode());
            put(values, "dh_camera_quality_dropoff", graphics.useCameraPositionForQualityDropOff());
            put(values, "dh_ambient_occlusion", graphics.ambientOcclusion().enabled());
            put(values, "dh_thread_count", configs.multiThreading().threadCount());
            put(values, "dh_runtime_ratio", configs.multiThreading().threadRuntimeRatio());
            values.put("dh_capture_status", "available");
            return values;
        } catch (RuntimeException | LinkageError unavailable) {
            // Missing/changed APIs must not look like disabled rendering or a zero distance.
            return Map.of("dh_capture_status", "unavailable");
        }
    }

    private static void put(Map<String, Object> target, String name, IDhApiConfigValue<?> setting) {
        Object value = setting.getValue(); // Includes active overrides; getTrueValue reads only the user base.
        if (value == null) throw new IllegalStateException("Unavailable configuration value");
        target.put(name, value instanceof Enum<?> choice ? choice.name() : value);
    }
}
