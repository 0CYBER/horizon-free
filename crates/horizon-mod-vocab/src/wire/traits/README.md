# Wire call-table roles

New interface traits declare their role in one file per root-exported trait:
`TraitName.import` for a trait a mod calls, or `TraitName.export` for a callback
implemented by a mod. File contents explain the role; the filename is the
machine-consumed identity. Duplicate roles and invalid names fail generation.
The generator resolves the name to a real public API trait and builds both sides
of its call table. Adding or removing a role file reruns generation.

This does not define a value layout. Public data types explicitly derive Wire.
Callback trait objects use the existing shared resource codec at their own
trait declaration, because traits cannot derive Wire. Do not append new traits
to the legacy import, export or handler lists.
