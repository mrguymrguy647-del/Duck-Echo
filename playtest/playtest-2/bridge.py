#!/usr/bin/env python3
"""A tiny MCP client for playtest 2.

It starts the real Duck Echo MCP server (target/release/duck-echo-mcp)
with its root set to the repository's playtest/ folder, exactly as the
project's .mcp.json does, sends one request over stdio, and prints the
answer. Every call is logged to calls.jsonl next to this file, so the
review can count how many tries each sound took.

Usage:
  python3 bridge.py list                     # server instructions and every tool
  python3 bridge.py call TOOL '{"arg": ...}' # call one tool, print its result

Images returned by a tool (picture: true) are saved as PNG files under
pictures/ next to this file, and their paths are printed.
"""
import base64
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
SERVER = os.path.join(REPO, "target", "release", "duck-echo-mcp")
ROOT = os.path.join(REPO, "playtest")


def session(requests):
    """Run the server for one exchange and return its replies by id."""
    init = {"jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "playtest-2-bridge", "version": "1"}}}
    done = {"jsonrpc": "2.0", "method": "notifications/initialized"}
    lines = [json.dumps(init), json.dumps(done)] + [json.dumps(r) for r in requests]
    out = subprocess.run([SERVER, "--root", ROOT], input="\n".join(lines) + "\n",
                         capture_output=True, text=True, timeout=600)
    replies = {}
    for line in out.stdout.splitlines():
        if line.strip():
            msg = json.loads(line)
            replies[msg.get("id")] = msg
    return replies


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "list":
        r = session([{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}])
        print("SERVER INSTRUCTIONS:\n" + r[0]["result"]["instructions"] + "\n")
        for t in r[1]["result"]["tools"]:
            print("=== " + t["name"] + "\n" + t["description"])
            print("arguments: " + json.dumps(t["inputSchema"]["properties"], indent=1) + "\n")
        return
    if len(sys.argv) < 3 or sys.argv[1] != "call":
        print(__doc__)
        sys.exit(2)
    tool = sys.argv[2]
    args = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
    r = session([{"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                  "params": {"name": tool, "arguments": args}}])[1]
    if "error" in r:
        result, is_error = r["error"], True
        print(json.dumps(r["error"], indent=1))
    else:
        res = r["result"]
        is_error = res.get("isError", False)
        print(("CALL FAILED (isError true)\n" if is_error else "") + res["content"][0]["text"])
        for block in res["content"][1:]:
            if block.get("type") == "image":
                os.makedirs(os.path.join(HERE, "pictures"), exist_ok=True)
                name = "%s-%d.png" % (tool, int(time.time() * 1000))
                path = os.path.join(HERE, "pictures", name)
                with open(path, "wb") as f:
                    f.write(base64.b64decode(block["data"]))
                print("\n[picture saved: %s]" % path)
    with open(os.path.join(HERE, "calls.jsonl"), "a") as log:
        log.write(json.dumps({"time": round(time.time(), 1), "tool": tool,
                              "arguments": args, "is_error": is_error}) + "\n")


if __name__ == "__main__":
    main()
