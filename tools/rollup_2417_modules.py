#!/usr/bin/env python3
"""Extract preset additions into sibling modules without moving game areas."""
from pathlib import Path
root = Path('crates/horizon-client/src')
def cut(path, start, end, target, replacement, prefix='', suffix=''):
    p = root / path
    s = p.read_text(); a = s.index(start); b = s.index(end, a)
    dest = root / target; dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(prefix + s[a:b] + suffix)
    p.write_text(s[:a] + replacement + s[b:])
cut('save.rs', '/// Saved preset/editor stems', '/// The world', 'save/preset_saved.rs', 'include!("save/preset_saved.rs");\n') if False else None
p=root/'save.rs';s=p.read_text();a=s.index('/// Saved preset/editor stems');b=s.index('\n}\n',a)+3
q=root/'save/preset_saved.rs';q.write_text(s[a:b]);p.write_text(s[:a]+'include!("save/preset_saved.rs");\n'+s[b:])
cut('save.rs','/// Structural boundary checks;', '#[derive(Clone, Debug, Default)]\npub struct Level', 'save/preset_generation.rs', 'include!("save/preset_generation.rs");\n')
cut('realms.rs','/// What the Create New World screen decided,','impl TemplateGroup {','realms/preset_world.rs','include!("realms/preset_world.rs");\n\n')
cut('title/tests.rs','#[test]\nfn bundled_mod_world_presets','mod multiplayer_parent_navigation {','title/preset_tests.rs','include!("preset_tests.rs");\n\n')
cut('title.rs','    fn buttons(&self, w: f32, h: f32) -> Vec<Button> {','    fn narration(&self)', 'title/preset_buttons.rs','', 'use super::*;\nimpl Title {\n','}\n')
p=root/'title.rs';s=p.read_text();s='#[path = "title/preset_buttons.rs"]\nmod preset_buttons;\n'+s if not s.startswith('//!') else s
# Keep inner module documentation first.
if 'mod preset_buttons;' not in s:
    pos=s.index('\nuse ');s=s[:pos]+'\n#[path = "title/preset_buttons.rs"]\nmod preset_buttons;\n'+s[pos:]
p.write_text(s)
# Native hooks shared.rs is included in native_hooks, so the extracted impl shares its scope.
p=root/'game/native_hooks/shared.rs';s=p.read_text();a=s.index('    pub(super) fn show_title_with_registry');b=s.index('\n    pub',a+10) if '\n    pub' in s[a+10:] else -1
# Locate the next method including any documentation instead of splitting its attributes.
depth=0;opened=False;end=None
for i in range(s.index('{',a),len(s)):
    if s[i]=='{':depth+=1;opened=True
    elif s[i]=='}':
        depth-=1
        if opened and depth==0:end=i+1;break
(root/'game/native_hooks/preset_title.rs').write_text('impl Game<\'_> {\n'+s[a:end]+'\n}\n')
p.write_text(s[:a]+s[end:])
p=root/'game/native_hooks/mod.rs';s=p.read_text().replace('include!("shared.rs");','include!("shared.rs");\ninclude!("preset_title.rs");');p.write_text(s)
