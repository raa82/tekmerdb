"""
agent_rag.py — Energy Compliance Analyst, RAG + Chroma backend.

Spawns chroma-mcp (official Chroma MCP server) as a stdio subprocess,
connects it to the chromadb container, then runs the ReAct agent loop
using Ollama as the reasoning engine.

Called by server.py — not run directly.
"""

import asyncio
import json
import logging
import os
import urllib.request
import urllib.error
from datetime import datetime
from pathlib import Path

from mcp_client import MCPClient
from ollama_client import run_agent_loop, OLLAMA_URL, OLLAMA_MODEL

log = logging.getLogger("agent_rag")

CHROMA_HOST = os.environ.get("CHROMA_HOST", "chromadb")
CHROMA_PORT = os.environ.get("CHROMA_PORT", "8000")
LOG_DIR     = os.environ.get("LOG_DIR", "/app/logs")

# chroma-mcp command — spawned via uvx
# chroma-mcp installed via pip, available as a script in PATH
CHROMA_MCP_CMD = [
    "chroma-mcp",
    "--client-type", "http",
    "--host",        CHROMA_HOST,
    "--port",        CHROMA_PORT,
    "--ssl",         "false",
]

SYSTEM_PROMPT = """You are an Energy Compliance Analyst at a European grid operator.
Your role is to answer compliance and regulatory questions using a knowledge base
built from the World Energy Outlook 2025 report.

KNOWLEDGE BASE — MANDATORY INSTRUCTIONS:
- The ONLY tool you may use to search is: chroma_query_documents
- The collection_name parameter MUST always be exactly: world_energy_outlook_2025
- You MUST call chroma_query_documents before answering any question
- Never answer from your own knowledge — only from retrieved documents

Example of the ONLY correct first action:
chroma_query_documents(collection_name="world_energy_outlook_2025", query="your search terms", n_results=5)

Key behaviours:
- First action is always chroma_query_documents — no exceptions
- Cite the source field from retrieved documents in your answer
- If retrieved information is insufficient, say so explicitly
- For EU AI Act compliance questions, be rigorous about data traceability
- Recommend human expert review when data is contradictory or sparse
- Be concise and structured. Bullet points for multi-part answers
- Stay in scope: energy, infrastructure, compliance only
"""


class RagAgent:
    """
    Manages the chroma-mcp subprocess lifecycle and exposes a single
    ask() coroutine used by server.py.
    """

    def __init__(self):
        self._mcp:   MCPClient | None = None
        self._tools: list[dict]       = []
        self._ready: bool             = False
        Path(LOG_DIR).mkdir(parents=True, exist_ok=True)

    async def _wait_for_chroma(self, retries: int = 30, delay: float = 3.0):
        """Poll Chroma until it responds before spawning chroma-mcp."""
        url = f"http://{CHROMA_HOST}:{CHROMA_PORT}/api/v2/heartbeat"
        log.info(f"Waiting for Chroma at {url}...")
        for attempt in range(retries):
            try:
                with urllib.request.urlopen(url, timeout=3) as resp:
                    if resp.status == 200:
                        log.info("Chroma is ready")
                        return
            except Exception as e:
                log.info(f"Chroma not ready (attempt {attempt + 1}/{retries}): {e}")
            await asyncio.sleep(delay)
        raise RuntimeError(f"Chroma did not become ready after {retries} attempts")

    async def start(self):
        await self._wait_for_chroma()
        log.info(f"Starting chroma-mcp → chromadb at {CHROMA_HOST}:{CHROMA_PORT}")
        self._mcp = MCPClient(command=CHROMA_MCP_CMD, label="chroma-mcp")
        await self._mcp.start()
        # Use a hardcoded clean schema for chroma_query_documents
        # chroma-mcp's native schema has anyOf/null patterns Groq rejects
        self._tools = [{
            "name": "chroma_query_documents",
            "description": "Search the World Energy Outlook 2025 knowledge base for relevant facts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "collection_name": {
                        "type": "string",
                        "description": "Always use: world_energy_outlook_2025"
                    },
                    "query_texts": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "List of search queries"
                    },
                    "n_results": {
                        "type": "integer",
                        "description": "Number of results to return (default 5)"
                    }
                },
                "required": ["collection_name", "query_texts"]
            }
        }]
        log.info(f"chroma-mcp ready — exposing 1 tool with clean schema")
        self._ready = True

    async def stop(self):
        if self._mcp:
            await self._mcp.stop()

    async def ask(self, question: str) -> dict:
        if not self._ready:
            return {"error": "agent not ready"}

        tool_calls_log = []
        ts_start       = asyncio.get_event_loop().time()

        async def call_tool(name: str, arguments: dict) -> str:
            result = await self._mcp.call_tool(name, arguments)
            tool_calls_log.append({"tool": name, "args": arguments, "result": result[:600]})
            log.info(f"tool call: {name}({json.dumps(arguments)}) → {len(result)} chars")
            return result

        try:
            answer = await run_agent_loop(
                question     = question,
                system       = SYSTEM_PROMPT,
                tools        = self._tools,
                call_tool_fn = call_tool,
            )
        except Exception as e:
            log.exception("Agent loop error")
            answer = f"Agent error: {e}"

        elapsed = round(asyncio.get_event_loop().time() - ts_start, 2)

        entry = {
            "ts":         datetime.utcnow().isoformat(),
            "backend":    "rag-chroma",
            "question":   question,
            "answer":     answer,
            "tool_calls": tool_calls_log,
            "elapsed_s":  elapsed,
        }
        log_path = Path(LOG_DIR) / f"rag_{datetime.utcnow().strftime('%Y%m%d')}.jsonl"
        with open(log_path, "a") as f:
            f.write(json.dumps(entry) + "\n")

        return {
            "answer":     answer,
            "tool_calls": tool_calls_log,
            "elapsed_s":  elapsed,
            "backend":    "rag-chroma",
        }