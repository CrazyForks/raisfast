#!/usr/bin/env python3
"""Tiny MCP stdio server (JSON-RPC over newline-delimited lines) for smoke tests.

Implements initialize / notifications/initialized / tools/list / tools/call.
Speaks only to the minimal subset raisfast's MCP client uses.
"""
import json
import sys


def reply(msg):
    if msg is None:
        return
    if isinstance(msg, dict):
        sys.stdout.write(json.dumps(msg) + "\n")
        sys.stdout.flush()


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        method = req.get("method", "")
        rid = req.get("id")
        if method == "initialize":
            reply({"jsonrpc": "2.0", "id": rid, "result": {
                "protocolVersion": req.get("params", {}).get("protocolVersion", "2024-11-05"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "echo", "version": "1.0"},
            }})
        elif method == "notifications/initialized":
            continue
        elif method == "tools/list":
            reply({"jsonrpc": "2.0", "id": rid, "result": {"tools": [
                {"name": "echo",
                 "description": "Echoes back the `msg` argument",
                 "inputSchema": {"type": "object",
                                 "properties": {"msg": {"type": "string"}},
                                 "required": ["msg"]}},
            ]}})
        elif method == "tools/call":
            params = req.get("params", {})
            name = params.get("name")
            args = params.get("arguments", {}) or {}
            if name == "echo":
                text = "echo:" + str(args.get("msg", ""))
            else:
                text = f"unknown tool {name}"
            reply({"jsonrpc": "2.0", "id": rid, "result": {
                "content": [{"type": "text", "text": text}],
                "isError": False,
            }})
        else:
            reply({"jsonrpc": "2.0", "id": rid,
                   "error": {"code": -32601, "message": "not implemented"}})


if __name__ == "__main__":
    main()
