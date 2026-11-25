#!/usr/bin/env python3
"""A short stream session against the render server, as a host would run it (PROTOCOL.md):
hello -> policy/resize/visible -> ready -> frame -> a control request on the same session -> a
click from the page -> a frame that read it -> (with --cli) the MCP bridge, `cadcraft-cli mcp
--connect`, drawing on the same session -> Save As from the menu answered over the wire -> stop ->
bye. Standard library only. Exit 0 when every step held; the frames it got go to OUT.

usage: stream_smoke.py BINARY OUT [--adapter auto|software] [--cli CADCRAFT_CLI]
"""

import json
import os
import secrets
import socket
import struct
import subprocess
import sys
import threading
import time

STEP_TIMEOUT = 180.0  # software adapters (lavapipe, WARP) build their pipelines slowly


def fail(msg, proc=None):
    print(f"smoke: FAIL: {msg}", flush=True)
    if proc is not None:
        proc.kill()
    sys.exit(1)


def read_exact(conn, n):
    buf = b""
    while len(buf) < n:
        chunk = conn.recv(n - len(buf))
        if not chunk:
            raise EOFError("leg A closed")
        buf += chunk
    return buf


def read_msg(conn):
    (length,) = struct.unpack("<I", read_exact(conn, 4))
    env = read_exact(conn, length)
    (hlen,) = struct.unpack("<I", env[:4])
    header = json.loads(env[4 : 4 + hlen].decode("utf-8"))
    return header, env[4 + hlen :]


def send_msg(conn, header, payload=b""):
    h = json.dumps(header).encode("utf-8")
    env = struct.pack("<I", len(h)) + h + payload
    conn.sendall(struct.pack("<I", len(env)) + env)


class Control:
    """The control channel, JSON lines with the token on each request."""

    def __init__(self, port, token):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=STEP_TIMEOUT)
        self.file = self.sock.makefile("rwb")
        self.token = token
        self.next = 1

    def call(self, method, params=None):
        req = {"id": self.next, "method": method, "params": params or {}, "token": self.token}
        self.next += 1
        self.file.write((json.dumps(req) + "\n").encode("utf-8"))
        self.file.flush()
        line = self.file.readline()
        if not line:
            raise EOFError("control channel closed")
        return json.loads(line)


class Mcp:
    """`cadcraft-cli mcp --connect`: MCP JSON-RPC over its stdio, one message per line."""

    def __init__(self, cli, port, log):
        self.proc = subprocess.Popen([cli, "mcp", "--connect", f"127.0.0.1:{port}"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log)
        self.next = 1

    def request(self, method, params):
        rid = self.next
        self.next += 1
        self.proc.stdin.write((json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params}) + "\n").encode("utf-8"))
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise EOFError("the MCP bridge closed its output")
            msg = json.loads(line)
            if msg.get("id") == rid:
                return msg

    def notify(self, method):
        self.proc.stdin.write((json.dumps({"jsonrpc": "2.0", "method": method}) + "\n").encode("utf-8"))
        self.proc.stdin.flush()

    def close(self):
        self.proc.stdin.close()
        try:
            self.proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self.proc.kill()


def main():
    if len(sys.argv) < 3:
        print(__doc__)
        sys.exit(2)
    binary, out = sys.argv[1], sys.argv[2]
    adapter = "auto"
    if "--adapter" in sys.argv:
        adapter = sys.argv[sys.argv.index("--adapter") + 1]
    cli = sys.argv[sys.argv.index("--cli") + 1] if "--cli" in sys.argv else None
    os.makedirs(out, exist_ok=True)
    render_token, control_token = secrets.token_hex(32), secrets.token_hex(32)
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(60)
    port = listener.getsockname()[1]
    env = dict(os.environ, CADCRAFT_RENDER_TOKEN=render_token, CADCRAFT_CONTROL_TOKEN=control_token, CADCRAFT_RENDER_LOG="info")
    log = open(os.path.join(out, "render-server.log"), "wb")
    w, h = 960, 600
    args = [binary, "--stream", f"127.0.0.1:{port}", "--session", "smoke", "--size", f"{w}x{h}", "--dpr", "1", "--lang", "en", "--adapter", adapter]
    proc = subprocess.Popen(args, env=env, stdout=log, stderr=subprocess.STDOUT)
    try:
        conn, _ = listener.accept()
    except socket.timeout:
        fail("the render server never connected", proc)
    conn.settimeout(STEP_TIMEOUT)

    # Messages arrive on a reader thread, so a step can wait for one kind while others go by.
    inbox, lock, cond = [], threading.Lock(), threading.Condition(threading.Lock())

    def reader():
        try:
            while True:
                m = read_msg(conn)
                with cond:
                    inbox.append(m)
                    cond.notify_all()
        except Exception as e:  # noqa: BLE001 - the end of leg A, however it ends
            with cond:
                inbox.append(({"k": "_closed", "why": str(e)}, b""))
                cond.notify_all()

    threading.Thread(target=reader, daemon=True).start()
    acked = set()

    def wait_for(pred, what, timeout=STEP_TIMEOUT):
        deadline = time.time() + timeout
        with cond:
            while True:
                for i, (hdr, payload) in enumerate(inbox):
                    if hdr.get("k") == "frame" and hdr["seq"] not in acked:
                        acked.add(hdr["seq"])
                        send_msg(conn, {"k": "ack", "seq": hdr["seq"], "decodeMs": 1.0})
                    if pred(hdr):
                        del inbox[: i + 1]
                        return hdr, payload
                    if hdr.get("k") in ("_closed", "error") or (hdr.get("k") == "bye" and what != "bye"):
                        fail(f"waiting for {what}: got {hdr}", proc)
                left = deadline - time.time()
                if left <= 0:
                    fail(f"no {what} in {timeout:.0f} s", proc)
                cond.wait(min(left, 1.0))

    def save_frame(hdr, payload, name):
        for i, r in enumerate(hdr["rects"]):
            ext = {"jpeg": "jpg", "png": "png", "rgba": "rgba"}[r["codec"]]
            with open(os.path.join(out, f"{name}-{i}.{ext}"), "wb") as f:
                f.write(payload[r["offset"] : r["offset"] + r["length"]])

    t0 = time.time()
    hello, _ = wait_for(lambda m: m.get("k") == "hello", "hello", 60)
    if hello.get("v") != 1 or hello.get("token") != render_token:
        fail(f"bad hello: {hello}", proc)
    print(f"smoke: hello from pid {hello['pid']}, {hello['backend']} {hello['adapter']} (software {hello['software']}), version {hello['version']}")
    send_msg(conn, {"k": "policy", "readOnly": None})
    send_msg(conn, {"k": "resize", "w": w, "h": h, "dpr": 1.0})
    send_msg(conn, {"k": "visible", "visible": True})
    ready, _ = wait_for(lambda m: m.get("k") == "ready", "ready")
    print(f"smoke: ready after {time.time() - t0:.1f} s: control port {ready['controlPort']}, {ready['w']}x{ready['h']} @ {ready['dpr']}")
    first, payload = wait_for(lambda m: m.get("k") == "frame", "the first frame")
    if not first["full"] or (first["w"], first["h"]) != (w, h):
        fail(f"the first frame is not a full {w}x{h} one: {first}", proc)
    save_frame(first, payload, "frame-first")
    print(f"smoke: frame {first['seq']} full, {len(first['rects'])} rects, {len(payload)} bytes, timing {first['timing']}")

    # The model's channel drives the session the user sees: a line drawn over it shows in a frame.
    ctl = Control(ready["controlPort"], control_token)
    pong = ctl.call("ping")
    if not pong.get("ok"):
        fail(f"ping: {pong}", proc)
    drawn = ctl.call("engine.execute", {"command": "line", "params": {"points": [[0, 0], [100, 60]]}})
    if not drawn.get("ok"):
        fail(f"engine.execute line: {drawn}", proc)
    inspected = ctl.call("drawing.inspect", {"limit": 5})
    print(f"smoke: control ping {pong.get('result')!r}, line drawn, drawing.inspect ok={inspected.get('ok')}")
    after_line, payload = wait_for(lambda m: m.get("k") == "frame", "a frame after the control request")
    print(f"smoke: frame {after_line['seq']} after the control request ({len(after_line['rects'])} rects)")

    # One click from the page, in the drawing area: the frame that read it says so.
    x, y = w * 0.55, h * 0.45
    mods = {"alt": False, "ctrl": False, "shift": False, "meta": False}
    events = [
        {"e": "move", "x": x, "y": y},
        {"e": "button", "x": x, "y": y, "button": "primary", "pressed": True, "mods": mods},
        {"e": "button", "x": x, "y": y, "button": "primary", "pressed": False, "mods": mods},
        {"e": "move", "x": x + 40, "y": y + 25},
    ]
    send_msg(conn, {"k": "input", "seq": 1, "events": events})
    clicked, payload = wait_for(lambda m: m.get("k") == "frame" and m.get("inputSeq", 0) >= 1, "a frame that read the click")
    save_frame(clicked, payload, "frame-after-click")
    print(f"smoke: frame {clicked['seq']} read input {clicked['inputSeq']} ({len(clicked['rects'])} rects)")
    send_msg(conn, {"k": "input", "seq": 2, "events": [{"e": "key", "key": "Escape", "physical": "Escape", "pressed": True, "repeat": False, "mods": mods}]})

    if cli:
        # The model's own bridge, unmodified: it reads the token file for this port and drives the
        # same session; what it draws shows in the pane.
        mcp = Mcp(cli, ready["controlPort"], open(os.path.join(out, "mcp-bridge.log"), "wb"))
        init = mcp.request("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "stream-smoke", "version": "1"}})
        if "result" not in init:
            fail(f"MCP initialize: {init}", proc)
        mcp.notify("notifications/initialized")
        tools = mcp.request("tools/list", {})
        names = [t["name"] for t in tools.get("result", {}).get("tools", [])]
        call = mcp.request("tools/call", {"name": "execute", "arguments": {"command": "circle", "params": {"center": [40, 30], "radius": 12}}})
        if "result" not in call or call["result"].get("isError"):
            fail(f"MCP execute circle: {call}", proc)
        seen = ctl.call("drawing.inspect", {"limit": 50})
        text = json.dumps(seen)
        if "Circle" not in text and "circle" not in text:
            fail(f"the bridge's circle is not in the session the pane shows: {text[:400]}", proc)
        after_mcp, payload = wait_for(lambda m: m.get("k") == "frame", "a frame after the MCP call")
        save_frame(after_mcp, payload, "frame-after-mcp")
        mcp.close()
        print(f"smoke: MCP bridge {init['result'].get('serverInfo', {})}, {len(names)} tools; its circle is in the pane's session (frame {after_mcp['seq']})")

    # Save As from the menu: the dialog is the host's (ask_save), the file goes where it answers.
    invoked = ctl.call("ui.menu.invoke", {"command": "ui.saveas"})
    if not invoked.get("ok"):
        fail(f"ui.menu.invoke ui.saveas: {invoked}", proc)
    ask, _ = wait_for(lambda m: m.get("k") == "ask_save", "ask_save")
    target = os.path.join(os.path.abspath(out), "smoke-" + os.path.basename(ask["name"]))
    send_msg(conn, {"k": "answer", "ask": ask["ask"], "path": target})
    done, _ = wait_for(lambda m: m.get("k") == "ask_done" and m.get("ask") == ask["ask"], "ask_done")
    if not done["ok"] or not os.path.isfile(target) or os.path.getsize(target) == 0:
        fail(f"Save As over the wire: {done}, file {target} exists={os.path.isfile(target)}", proc)
    print(f"smoke: ask_save {ask['name']} {ask['exts']} answered; ask_done ok, {os.path.getsize(target)} bytes written")

    send_msg(conn, {"k": "stop", "why": "smoke done"})
    bye, _ = wait_for(lambda m: m.get("k") == "bye", "bye", 30)
    if bye.get("why") != "stop":
        fail(f"bye: {bye}", proc)
    try:
        code = proc.wait(timeout=15)
    except subprocess.TimeoutExpired:
        fail("did not exit after bye", proc)
    if code != 0:
        fail(f"exit code {code}")
    print(f"smoke: bye stop, exit 0; {time.time() - t0:.1f} s in all")


if __name__ == "__main__":
    main()
