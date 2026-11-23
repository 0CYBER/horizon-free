"""`worldborder float center`: `/worldborder center` works on the float position `Vec2Argument` gives."""

GROUP = 'worldborder float center'


def cases(cmd, setup):
    """`Vec2Argument.getVec2` narrows each coordinate to a float. `WorldBorderCommand.setCenter` compares
    that float with the current centre, tests `Math.abs` of it against 2.9999984E7, stores it widened to a
    double and prints it with `%.2f`. Coordinates carry a decimal point, so no block-centre correction
    applies. Each far centre is put back at once."""
    return [
        cmd('/worldborder center 0.0 0.0'),
        # 0.1 is stored as the float 0.100000001490116...: the double below is the same centre.
        cmd('/worldborder center 0.1 0.0'),
        cmd('/worldborder center 0.10000000149011612 0.0'),
        # Rounded with %.2f from the float: 1.0049999952316284 and -2.674999952316284.
        cmd('/worldborder center 1.005 -2.675'),
        cmd('/execute positioned 2.675 0 1.005 run worldborder center ~ ~'),
        # Past the limit as a double, on it as a float (29999984.0); a control past it either way.
        cmd('/worldborder center 29999984.5 0.0'),
        cmd('/worldborder center 0.0 0.0'),
        cmd('/worldborder center 0.0 -29999985.0'),
        cmd('/worldborder center 0.0 0.0'),
        cmd('/worldborder center 29999986.0 0.0'),
    ]
