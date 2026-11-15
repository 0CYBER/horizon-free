# Frame identity: entity stage beside a traced material

A test fixture of `tools/test_frame_identity_parity.py` (session `entity_traced`), not a look and not offered to
players: a shader API 0.2 pack whose `[material]` traces rays on translucent terrain and whose `[entity]` stage tints
the entities. Where the device traces, the material's pipelines read the frame's group joined with the ray scene,
and the entity stage's read the frame's own group; translucent terrain drawn after the entities must find the
material's group bound again. Check it with `horizon-shader check tools/fixtures/shaderpacks/entity_traced`.
