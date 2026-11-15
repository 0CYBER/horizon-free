"""Pure move of the projectile damage adapter out of the oversized mobs module."""
from pathlib import Path

root = Path(__file__).resolve().parents[1]
source = root / "crates/horizon-client/src/mobs.rs"
text = source.read_text()
start = text.index('    /// `damage` of a blow dealt by a projectile')
end = text.index('    /// `hurtServer` of `amount`', start)
block = text[start:end]
text = text[:start] + text[end:]
text += '\nmod projectile_damage;\n'
source.write_text(text)
(root / 'crates/horizon-client/src/mobs/projectile_damage.rs').write_text(
    'use super::{Aim, Damage, DamageLevel, Mob};\n\nimpl Mob {\n' + block + '}\n'
)
