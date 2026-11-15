# Client effect interpolation

`LivingRef::effect_blend_factor(host, effect, partial)` reads the local player's client effect blend.
A newly blended ordinary effect reads 0 before its first effect tick, then interpolates to its
post-countdown target each client tick: 1 while active, 0 once expired. A finite effect that expires
on its first tick never rises; an infinite effect has the initial interpolation.
An unblended addition starts at its target immediately; replacing an existing effect keeps its blend.
Darkness and nausea use longer fades. Pass the render partial tick unchanged: the read
does not clamp it. A missing effect reads `Some(0.0)`; an unknown effect, stale handle or entity other
than the local player reads `None`.
