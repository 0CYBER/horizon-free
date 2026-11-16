# Raid handles

Use `LevelRef::raid`, `raid_at` and `raids` to obtain raid handles. They resolve their
own loaded server level even while another dimension is in place. Raid reads,
omen-level writes and `stop` do not swap levels. Raider lists and living health
come from that level's mobs; raid ids are local to a level. A closed level, an
earlier world epoch or a client refuses the handle. Entity handles in a parked
raid's member list keep their identity; their ordinary entity operations
require the entity's level to be in place.
