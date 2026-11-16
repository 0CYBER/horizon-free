#!/usr/bin/env python3
"""Mechanical first pass for the owned Guest inbox API (run from repo root).

Hand corrections for dispatch, filtering and resumable drain follow separately.
Only synthetic tuple constructors/patterns and the owned tuple type migrate here.
"""
from pathlib import Path

ROOT = Path("crates/horizon-client/src")
FILES = ["lan/guest.rs", "lan/hub.rs", "lan/guest_list.rs", "lan/arrivals.rs",
         "game/interface/play_ping_tests.rs", "game/network/lan_actions/tests/boat_holder_movement_tests.rs"]
for name in FILES:
    path = ROOT / name
    source = path.read_text()
    source = source.replace("VecDeque<(play::Serverbound, usize)>", "VecDeque<InboxItem>")
    source = source.replace("std::collections::VecDeque<(horizon_protocol::packets::play::Serverbound, usize)>", "std::collections::VecDeque<super::guest::InboxItem>")
    # Every tuple push_back in these files is a Guest inbox enqueue. Preserve
    # the old synthetic size argument temporarily; hand fixes replace real
    # enqueues with owned provider credits and remove wire-size bookkeeping.
    source = source.replace("push_back((", "push_back(horizon_client::lan::guest::InboxItem::synthetic(")
    path.write_text(source)
