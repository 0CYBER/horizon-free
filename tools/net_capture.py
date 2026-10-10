#!/usr/bin/env python3
"""A decoding TCP proxy for the Minecraft 26.3 protocol (offline mode, no
encryption): clients connect to LISTEN, it forwards to TARGET and records
every packet in both directions as JSON lines (time, direction, state, id,
name, size and the payload), following the connection state through the
handshake, login, configuration and play (and back to configuration).

  net_capture.py --listen 25570 --target localhost:25565 --out capture.jsonl

The packet names come from crates/horizon-protocol/src/ids.rs, which is
generated from vanilla's packets report. `summary` prints what a capture
contains, `diff` compares the packet sequences of two captures (for example
Horizon hosting and a vanilla server serving the same world).
"""
import argparse
import json
import re
import socket
import sys
import threading
import time
import zlib
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
STATES = ["handshake", "status", "login", "configuration", "play"]


def load_names(path=ROOT / "crates/horizon-protocol/src/ids.rs"):
    """{(state, direction): [name by id]} from the generated id tables."""
    text = path.read_text()
    names = {}
    for state in STATES:
        block = re.search(r"pub mod %s \{(.*?)\n\}" % state, text, re.S)
        if not block:
            continue
        for direction in ("clientbound", "serverbound"):
            part = re.search(r"pub mod %s \{.*?NAMES: &\[&str\] = &\[(.*?)\];" % direction,
                             block.group(1), re.S)
            if part:
                names[(state, direction)] = re.findall(r'"([^"]+)"', part.group(1))
    return names


def read_varint(data, pos):
    value = shift = 0
    while True:
        if pos >= len(data):
            raise IndexError("truncated varint")
        b = data[pos]
        pos += 1
        value |= (b & 0x7F) << shift
        shift += 7
        if not b & 0x80:
            if value & 0x80000000:
                value -= 1 << 32
            return value, pos
        if shift > 35:
            raise ValueError("varint too long")


class Session:
    """The state both directions of one connection share."""

    def __init__(self, conn_id, out, names, lock, full):
        self.id = conn_id
        self.state = {"clientbound": "handshake", "serverbound": "handshake"}
        self.threshold = -1
        self.out = out
        self.names = names
        self.lock = lock
        self.full = full
        self.start = time.time()
        self.closed = threading.Event()

    def name(self, direction, packet_id):
        table = self.names.get((self.state[direction], direction), [])
        return table[packet_id] if 0 <= packet_id < len(table) else f"unknown_{packet_id}"

    def record(self, direction, packet_id, payload):
        state = self.state[direction]
        name = self.name(direction, packet_id)
        entry = {
            "conn": self.id,
            "t": round(time.time() - self.start, 4),
            "dir": "s2c" if direction == "clientbound" else "c2s",
            "state": state,
            "id": packet_id,
            "name": name,
            "size": len(payload),
        }
        if self.full or len(payload) <= 4096:
            entry["data"] = payload.hex()
        else:
            entry["head"] = payload[:512].hex()
        with self.lock:
            self.out.write(json.dumps(entry) + "\n")
            self.out.flush()
        self.transition(direction, name, payload)

    def transition(self, direction, name, payload):
        s = self.state
        if direction == "serverbound":
            if s["serverbound"] == "handshake" and name == "intention":
                pos = read_varint(payload, 0)[1]
                length, pos = read_varint(payload, pos)
                intent = read_varint(payload, pos + length + 2)[0]
                nxt = {1: "status", 2: "login", 3: "login"}.get(intent, "login")
                s["serverbound"] = s["clientbound"] = nxt
            elif s["serverbound"] == "login" and name == "login_acknowledged":
                s["serverbound"] = s["clientbound"] = "configuration"
            elif s["serverbound"] == "configuration" and name == "finish_configuration":
                s["serverbound"] = s["clientbound"] = "play"
            elif s["serverbound"] == "play" and name == "configuration_acknowledged":
                s["serverbound"] = s["clientbound"] = "configuration"
        elif s["clientbound"] == "login" and name == "login_compression":
            self.pending_threshold = read_varint(payload, 0)[0]


class Pipe(threading.Thread):
    def __init__(self, session, direction, src, dst):
        super().__init__(daemon=True)
        self.session, self.direction, self.src, self.dst = session, direction, src, dst

    def run(self):
        buffer = b""
        s = self.session
        try:
            while True:
                chunk = self.src.recv(1 << 16)
                if not chunk:
                    break
                buffer += chunk
                frames = []
                while True:
                    try:
                        length, pos = read_varint(buffer, 0)
                    except IndexError:
                        break
                    if len(buffer) < pos + length:
                        break
                    frames.append(buffer[:pos + length])
                    body, buffer = buffer[pos:pos + length], buffer[pos + length:]
                    threshold = s.threshold
                    if threshold >= 0:
                        size, p = read_varint(body, 0)
                        body = zlib.decompress(body[p:]) if size else body[p:]
                    packet_id, p = read_varint(body, 0)
                    s.record(self.direction, packet_id, body[p:])
                    # The compression threshold applies after its packet, in
                    # both directions; the client answers only after reading it.
                    pending = getattr(s, "pending_threshold", None)
                    if pending is not None:
                        s.threshold = pending
                        s.pending_threshold = None
                    # State switches must be in effect before the peer can
                    # answer, so forward each frame after recording it.
                    self.dst.sendall(frames.pop())
        except (OSError, ValueError, zlib.error) as e:
            with s.lock:
                s.out.write(json.dumps({"conn": s.id, "event": "error", "dir": self.direction,
                                        "error": str(e)}) + "\n")
                s.out.flush()
        finally:
            for sock in (self.src, self.dst):
                try:
                    sock.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
            s.closed.set()


def serve(listen, target, out_path, full=False, ready=None, stop=None):
    names = load_names()
    host, port = target.rsplit(":", 1)
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind(("127.0.0.1", listen))
    server.listen()
    server.settimeout(0.5)
    lock = threading.Lock()
    conn_id = 0
    with open(out_path, "a") as out:
        if ready:
            ready.set()
        while not (stop and stop.is_set()):
            try:
                client, _ = server.accept()
            except socket.timeout:
                continue
            conn_id += 1
            try:
                upstream = socket.create_connection((host, int(port)))
            except OSError as e:
                client.close()
                with lock:
                    out.write(json.dumps({"conn": conn_id, "event": "error",
                                          "error": f"upstream: {e}"}) + "\n")
                continue
            for sock in (client, upstream):
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            session = Session(conn_id, out, names, lock, full)
            Pipe(session, "serverbound", client, upstream).start()
            Pipe(session, "clientbound", upstream, client).start()
    server.close()


def start_background(listen, target, out_path, full=False):
    """Run the proxy on a thread; returns the stop event."""
    ready, stop = threading.Event(), threading.Event()
    threading.Thread(target=serve, args=(listen, target, out_path, full, ready, stop),
                     daemon=True).start()
    ready.wait(5)
    return stop


def load(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]


def play_sessions(entries):
    """Connections that reached the play state."""
    return sorted({e["conn"] for e in entries if e.get("state") == "play"})


def summary(entries, conn=None):
    lines = []
    conns = [conn] if conn else sorted({e["conn"] for e in entries})
    for c in conns:
        mine = [e for e in entries if e["conn"] == c and "name" in e]
        lines.append(f"connection {c}: {len(mine)} packets")
        counts = Counter((e["dir"], e["state"], e["name"]) for e in mine)
        for (d, s, n), k in sorted(counts.items()):
            size = sum(e["size"] for e in mine if (e["dir"], e["state"], e["name"]) == (d, s, n))
            lines.append(f"  {d} {s:13} {n:40} x{k:<6} {size} bytes")
        for e in entries:
            if e["conn"] == c and e.get("event") == "error":
                lines.append(f"  error ({e.get('dir')}): {e['error']}")
    return "\n".join(lines)


def sequence(entries, conn, direction="s2c", until="player_position"):
    """Packet names in order, up to and including the first `until`."""
    out = []
    for e in entries:
        if e["conn"] != conn or e.get("dir") != direction or "name" not in e:
            continue
        out.append(f"{e['state']}/{e['name']}")
        if e["name"] == until and e["state"] == "play":
            break
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("proxy")
    p.add_argument("--listen", type=int, required=True)
    p.add_argument("--target", required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--full", action="store_true", help="keep every payload in full")
    s = sub.add_parser("summary")
    s.add_argument("capture", type=Path)
    d = sub.add_parser("diff")
    d.add_argument("a", type=Path)
    d.add_argument("b", type=Path)
    d.add_argument("--until", default="player_position")
    args = parser.parse_args()
    if args.command == "proxy":
        serve(args.listen, args.target, args.out, args.full)
    elif args.command == "summary":
        print(summary(load(args.capture)))
    else:
        import difflib
        a, b = load(args.a), load(args.b)
        sa = sequence(a, play_sessions(a)[0], until=args.until)
        sb = sequence(b, play_sessions(b)[0], until=args.until)
        sys.stdout.writelines(difflib.unified_diff([x + "\n" for x in sa], [x + "\n" for x in sb],
                                                   str(args.a), str(args.b)))


if __name__ == "__main__":
    main()
