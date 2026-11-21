"""The one Cargo selection the harnesses build their fast-profile programs with.

Cargo settles the features of shared dependencies over every package a command selects, so horizon-client built
under two selections is compiled twice, and the parity prebuild (tools/parity.py, prebuild_commands) makes one Cargo
run per selection. Every harness that builds horizon-client, horizon-ctl, horizon-server, horizon-official or
horizon-relay in the fast profile goes through `command`, naming only the programs it runs. Examples stay in Cargo
runs of their own: building one turns on horizon-client-lib's dev-dependencies, which compile test-only code into the
game.
"""

PACKAGES = ('horizon-game', 'horizon-client-lib', 'horizon-server', 'horizon-official', 'horizon-relay')


def command(*programs):
    """`cargo build` of these programs in the fast profile, with the shared selection."""
    return ['cargo', 'build', '--locked', '--profile', 'fast', *(part for package in PACKAGES for part in ('-p', package)),
            *(part for program in programs for part in ('--bin', program))]
