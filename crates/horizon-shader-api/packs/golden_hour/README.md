# Golden Hour

Horizon's example shader pack (docs/shaders.md): a sky stage and two post passes, with five options, in linear light
([shaders-reference-0.2.md](../../../../docs/api/shaders-reference-0.2.md)). The glow and the golden tint work on
light; the soft shoulder and the saturation are a look written for display values, so the grade runs them on the
encoded picture. It has no `[tonemap]` of its own: Horizon's curve shows light above white, in SDR a clip at white.
Check it with `horizon-shader check crates/horizon-shader-api/packs/golden_hour`.
