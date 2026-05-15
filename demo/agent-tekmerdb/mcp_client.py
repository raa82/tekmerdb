"""
mcp_client.py — Lightweight stdio MCP client.

Spawns an MCP server binary as a subprocess and speaks JSON-RPC over
stdin/stdout. This is exactly how Claude Desktop talks to MCP servers.

Used by both agents:
  agent_tekmerdb.py  →  MCPClient(["./pfodb-mcp"])
  agent_rag.py       →  MCPClient(["uvx", "chroma-mcp", ...])
"""

import asyncio
import json
import logging
import sys
from typing import Any

log = logging.getLogger("mcp_client")


class MCPError(Exception):
    pass


class MCPClient:

    def __init__(self, command: list[str], label: str = "mcp"):
        self.command = command
        self.label   = label
        self._proc   = None
        self._id     = 0
        self._lock   = asyncio.Lock()

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    async def start(self):
        log.info(f"[{self.label}] spawning: {' '.join(self.command)}")
        self._proc = await asyncio.create_subprocess_exec(
            *self.command,
            stdin  = asyncio.subprocess.PIPE,
            stdout = asyncio.subprocess.PIPE,
            stderr = asyncio.subprocess.PIPE,
        )
        asyncio.ensure_future(self._drain_stderr())
        await self._initialize()
        log.info(f"[{self.label}] handshake complete")

    async def stop(self):
        if self._proc and self._proc.returncode is None:
            self._proc.stdin.close()
            try:
                await asyncio.wait_for(self._proc.wait(), timeout=5.0)
            except asyncio.TimeoutError:
                self._proc.kill()
        log.info(f"[{self.label}] stopped")

    async def _drain_stderr(self):
        while True:
            line = await self._proc.stderr.readline()
            if not line:
                break
            sys.stderr.buffer.write(line)
            sys.stderr.buffer.flush()

    # ------------------------------------------------------------------
    # JSON-RPC
    # ------------------------------------------------------------------

    def _next_id(self) -> int:
        self._id += 1
        return self._id

    async def _send(self, method: str, params: dict | None = None) -> Any:
        async with self._lock:
            msg_id  = self._next_id()
            request = {"jsonrpc": "2.0", "id": msg_id, "method": method}
            if params is not None:
                request["params"] = params

            raw = json.dumps(request) + "\n"
            log.debug(f"[{self.label}] → {raw.strip()}")

            self._proc.stdin.write(raw.encode())
            await self._proc.stdin.drain()

            while True:
                line = await self._proc.stdout.readline()
                if not line:
                    raise MCPError("MCP server closed stdout unexpectedly")
                line = line.decode().strip()
                if not line:
                    continue
                log.debug(f"[{self.label}] ← {line}")

                try:
                    resp = json.loads(line)
                except json.JSONDecodeError:
                    log.warning(f"[{self.label}] non-JSON: {line[:80]}")
                    continue

                if resp.get("id") != msg_id:
                    continue  # notification or out-of-order

                if "error" in resp:
                    raise MCPError(f"MCP error: {resp['error']}")

                return resp.get("result")

    async def _notify(self, method: str, params: dict | None = None):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        raw = json.dumps(msg) + "\n"
        self._proc.stdin.write(raw.encode())
        await self._proc.stdin.drain()

    # ------------------------------------------------------------------
    # MCP protocol
    # ------------------------------------------------------------------

    async def _initialize(self):
        await self._send("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities":    {},
            "clientInfo":      {"name": "energy-compliance-agent", "version": "1.0.0"},
        })
        await self._notify("notifications/initialized")

    async def list_tools(self) -> list[dict]:
        result = await self._send("tools/list")
        return result.get("tools", [])

    async def call_tool(self, name: str, arguments: dict) -> str:
        """Call a tool and return the text content of the result."""
        result  = await self._send("tools/call", {"name": name, "arguments": arguments})
        content = result.get("content", [])
        parts   = [b.get("text", "") for b in content if b.get("type") == "text"]
        return "\n".join(parts)
