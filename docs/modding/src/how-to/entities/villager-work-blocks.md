# Villager secondary work blocks

Add blocks to a vanilla profession's secondary work sites with the block tag
`horizon:villager_secondary_poi/<profession>`. For example,
`data/horizon/tags/block/villager_secondary_poi/farmer.json`:

```json
{"values": ["minecraft:soul_sand"]}
```

Entries add to the profession's defaults. Farmers retain farmland, including
blocks using the farmland template. Other vanilla professions can use their own
names, such as `fisherman`. For a custom profession, declare
`ProfessionDefinition::secondary_blocks` through
[`register_villager_professions`](villager-professions.md). These blocks participate
in the same secondary-site scan.

Tags use the normal world-open and `/reload` tag resolution. Successful reloads
replace membership; failed reloads retain the previous world data. Removing all
entries restores vanilla behavior.

The server sensor scans inclusively within four blocks
in x and z and two in y. Professions with empty secondary sets skip the scan.
Results populate secondary job-site brain memory. The shared work activity can stroll toward a randomly selected secondary
site while near its primary job site. Farming requires a farmer and valid crops and farmland.
