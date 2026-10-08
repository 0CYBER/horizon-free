//! Entity, living and player damage-immunity predicate ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImmunityTag {
    BypassesInvulnerability,
    Fire,
    Fall,
    Drowning,
    Freezing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageRule {
    Drowning,
    Fall,
    Fire,
    Freeze,
}
pub trait ImmunityWorld {
    type Error;
    fn removed(&self) -> bool;
    fn permanently_invulnerable(&mut self) -> Result<bool, Self::Error>;
    fn temporarily_invulnerable(&mut self) -> Result<bool, Self::Error>;
    fn invulnerable(&mut self) -> Result<bool, Self::Error> {
        entity_invulnerable(self)
    }
    fn tag(&mut self, tag: ImmunityTag) -> Result<bool, Self::Error>;
    fn creative_source(&mut self) -> Result<bool, Self::Error>;
    fn fire_immune(&mut self) -> Result<bool, Self::Error>;
    fn fall_immune(&mut self) -> Result<bool, Self::Error>;
    fn enchantment_immunity(&mut self) -> Result<bool, Self::Error>;
    fn damage_rule(&mut self, rule: DamageRule) -> Result<bool, Self::Error>;
    fn changing_dimension(&mut self) -> Result<bool, Self::Error>;
    fn ender_pearl(&mut self) -> Result<bool, Self::Error>;
    fn client_loaded(&mut self) -> Result<bool, Self::Error>;
}
pub fn entity_invulnerable<W: ImmunityWorld + ?Sized>(w: &mut W) -> Result<bool, W::Error> {
    Ok(w.permanently_invulnerable()? || w.temporarily_invulnerable()?)
}
pub fn base_immunity<W: ImmunityWorld + ?Sized>(w: &mut W) -> Result<bool, W::Error> {
    if w.removed() {
        return Ok(true);
    }
    if w.invulnerable()? && !w.tag(ImmunityTag::BypassesInvulnerability)? && !w.creative_source()? {
        return Ok(true);
    }
    if w.tag(ImmunityTag::Fire)? && w.fire_immune()? {
        return Ok(true);
    }
    Ok(w.tag(ImmunityTag::Fall)? && w.fall_immune()?)
}
pub fn living_immunity<W: ImmunityWorld + ?Sized>(w: &mut W) -> Result<bool, W::Error> {
    Ok(base_immunity(w)? || w.enchantment_immunity()?)
}
pub fn player_immunity<W: ImmunityWorld + ?Sized>(w: &mut W) -> Result<bool, W::Error> {
    if living_immunity(w)? {
        return Ok(true);
    }
    for (tag, rule) in [
        (ImmunityTag::Drowning, DamageRule::Drowning),
        (ImmunityTag::Fall, DamageRule::Fall),
        (ImmunityTag::Fire, DamageRule::Fire),
        (ImmunityTag::Freezing, DamageRule::Freeze),
    ] {
        if w.tag(tag)? {
            return Ok(!w.damage_rule(rule)?);
        }
    }
    Ok(false)
}
pub fn server_player_immunity<W: ImmunityWorld + ?Sized>(w: &mut W) -> Result<bool, W::Error> {
    Ok(player_immunity(w)?
        || (w.changing_dimension()? && !w.ender_pearl()?)
        || !w.client_loaded()?)
}
