# Observing refused damage attempts

`entity_invulnerability_check` observes an attempted hit without changing its amount. It fires on
both sides (NeoForge's `EntityInvulnerabilityCheckEvent`): its level host says which one.
It supplies the entity, damage source, original invulnerability decision and a writable decision.
A listener that only plays a sound leaves the decision unchanged. There is no amount: vanilla's
`isInvulnerableTo` takes the level and damage source, not damage. Amount mutation belongs on
`living_incoming_damage`, which runs after the early living guards but before the hurt-cooldown
comparison.

For player hits, the observation is delivered once across the server-player, player and living
predicates and the paused equipment continuations. The base decision is retained for that attempt;
the surrounding predicates apply their own PvP, ability, gamerule, death and client-loaded
guards. A new hit, including one refused by cooldown, starts a new observation. Creative and
PvP-refused player wrappers do not suppress it. Ordinary mob hits also deliver it before death,
fire resistance, shield and cooldown refusal. Ghast and warden overrides deliver their complete
invulnerability decision, including flagged ghasts, reflected fireballs and digging wardens.

## Zero damage and projectile hits

Zero-damage snowball, egg, ender-pearl and custom thrown collisions enter the player
predicate before the zero-damage refusal. A successfully teleported player's pearl also attempts
its separate fall damage when a listener changes that damage to zero. Wither-skull impact and
its subsequent explosion are separate attempts. `/damage` asks the predicate even for spectators.

Firework damage to the attached entity is a direct attempt, including creative, spectator or dead
targets. Its splash query excludes spectators, but includes creative, dead and unloaded
resident players and dead resident mobs. Ability, death and client-loaded refusals occur inside
the damage pipeline, after the observation.

## Attempts and target selection

A damage-generating precondition is not itself an invulnerability query. In particular:

- Poison at health at or below one does not call vanilla `hurtServer` and produces no observation.
- Spectators are absent from vanilla's explosion entity query; team damage restrictions can also
  prevent an explosion damage call. Neither generates an observation. Creative players remain in
  the query and their attempted explosion damage is observed before ability refusal.
- Falling-block damage selects only living, alive, non-creative, non-spectator entities. Evoker
  fangs select living, alive entities. Pufferfish contact comes from the player's alive,
  non-spectator contact selection. Targets omitted by those vanilla queries have no attempt.
- A sulfur cube with a body item can consume body-immune damage in its `hurtServer` override,
  performing its special push and returning without entering the living invulnerability predicate.
  That path produces no observation, matching the absence of a vanilla predicate call. Ordinary
  sulfur cube damage that enters the living pipeline is observed.
- Missing targets and unknown resident damage-source identifiers do not create hit attempts.
- A client predicting the mount it rides asks the hook on the client side wherever that tick
  evaluates the predicate; the server still asks for its own tick.

Selected armor stand, item, vehicle and experience orb damage paths also fire
this hook, as do end crystal, item frame and painting damage paths. An explicit
invulnerability query (`EntityRef::invulnerable_to_damage`, a mod goal's
`invulnerable_to_damage`) asks the hook too, once per query, with a read-only level.
