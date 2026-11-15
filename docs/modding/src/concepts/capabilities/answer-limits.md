# Capability answer limits

Use `?` to propagate errors from `move_resources` and `move_energy` out of your
`host.transaction` closure. If a handler returns more than requested or offered,
the helper rolls back the offending subtransaction and returns `Overanswered`.
Propagating that error aborts the enclosing transaction, including its earlier moves.

- `Overanswered.owner` identifies the provider's mod, or `"the game"`.
- The game warns once, naming the offending provider.
- Your handler must return no more than the amount passed to `insert` or `extract`
  and journal its writes in the supplied transaction so rollback can undo them.
- A destination that accepts only part of a move causes a rollback and one retry
  with that smaller amount; partial acceptance is not an overanswer.
