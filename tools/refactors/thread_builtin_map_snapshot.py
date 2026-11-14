#!/usr/bin/env python3
"""One-shot migration: thread builtin maps through exhaustive MobWorld constructions.

Update expressions inherit the original view, never a vanilla default. The
hookless worker view binds and preserves the source snapshot explicitly.
"""
from pathlib import Path
import re
import subprocess

root = Path('crates/horizon-client')
if "pub mob_maps:" in (root / 'src/mobs.rs').read_text():
    raise SystemExit('snapshot threading already applied; run only on the migration parent')
paths = subprocess.check_output(['git', 'ls-files', str(root)], text=True).splitlines()
for filename in paths:
    if not filename.endswith('.rs'):
        continue
    path = Path(filename)
    source = path.read_text()
    edits = []
    for match in re.finditer(r'\bMobWorld\s*\{', source):
        start = match.end()
        depth, cursor, update = 1, start, False
        while depth:
            char = source[cursor]
            if depth == 1 and source[cursor:cursor+2] == '..' and (
                not source[start:cursor].strip() or source[start:cursor].rstrip().endswith(',')
                or not source[source.rfind('\n', 0, cursor)+1:cursor].strip()
            ):
                update = True
            depth += (char == '{') - (char == '}')
            cursor += 1
        body = source[start:cursor-1]
        if update or re.search(r'\bmob_maps\s*[:,]', body):
            continue
        indent = re.match(r'\s*\n( *)', body).group(1)
        if filename.endswith('/game/creatures/prep.rs'):
            field = 'mob_maps,'
        elif filename.endswith('/game/mobs/creatures/world_view.rs'):
            field = 'mob_maps: self.mob_maps(),'
        else:
            field = 'mob_maps: horizon_client::neoforge_mob_maps::NeoForgeMobMaps::vanilla(),'
        edits.append((start, '\n' + indent + field))
    for start, insertion in reversed(edits):
        source = source[:start] + insertion + source[start:]
    if edits:
        path.write_text(source)

path = root / 'src/mobs.rs'
source = path.read_text()
source = source.replace("pub struct MobWorld<'a> {", "pub struct MobWorld<'a> {\n    pub mob_maps: &'a crate::neoforge_mob_maps::NeoForgeMobMaps,")
source = source.replace('        Seen {\n', '        Seen {\n            kind: Arc::clone(&m.kind),\n', 1)
path.write_text(source)
path = root / 'src/mobs/seen.rs'
source = path.read_text().replace('pub struct Seen {', 'pub struct Seen {\n    /// Actual registry type, retained without copying its id string.\n    pub kind: Arc<MobType>,')
path.write_text(source)
