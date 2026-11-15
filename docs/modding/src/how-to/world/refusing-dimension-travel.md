# Refusing dimension travel

`entity_travel_to_dimension` runs before a server entity moves to the target
level. Return `Flow::Cancel` to refuse that entity's travel. Passengers are
asked recursively: refusing one leaves it dismounted in the source level,
with its own passengers, without refusing the vehicle's transition. Discarding
an entity through its handle during the callback also prevents its travel.
Callbacks may change riding relationships. An original passenger edge is restored
only if the checked mount operation still permits it; otherwise the passenger
stays behind, without overwriting any new vehicle chosen by a callback.
A falling block ticks outside the level's entity list, so its handle is not
accessible during this callback.
