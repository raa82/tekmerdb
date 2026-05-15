"""
ollama_client.py — LLM client supporting Groq API and Ollama.

Reads configuration from config.py in the demo/ root directory.
Groq is used when GROQ_API_KEY is set, Ollama as fallback.

For TekmerDB agent: single-pass approach — retrieval + reliability data
are combined into one prompt that produces the final formatted answer.

For RAG agent: standard ReAct loop with native tool calling.
"""

import json
import logging
import os
import sys
import requests
from typing import Callable, Awaitable

log = logging.getLogger("llm_client")

# =============================================================================
# Load config
# =============================================================================
def _load_config():
    # Look for config.py in demo/ root (parent of agent-rag/ or agent-tekmerdb/)
    search_paths = [
        os.path.join(os.path.dirname(__file__), "..", "config.py"),
        os.path.join(os.path.dirname(__file__), "config.py"),
        "config.py",
    ]
    for path in search_paths:
        path = os.path.abspath(path)
        if os.path.exists(path):
            cfg = {}
            with open(path) as f:
                exec(f.read(), cfg)
            return cfg
    return {}

_cfg = _load_config()

GROQ_API_KEY  = _cfg.get("GROQ_API_KEY", os.environ.get("GROQ_API_KEY", ""))
GROQ_BASE_URL = _cfg.get("GROQ_BASE_URL", "https://api.groq.com/openai/v1")
GROQ_MODEL    = _cfg.get("GROQ_MODEL", "llama-3.3-70b-versatile")
OLLAMA_URL    = _cfg.get("OLLAMA_URL", os.environ.get("OLLAMA_URL", "http://localhost:11434"))
OLLAMA_MODEL  = _cfg.get("OLLAMA_MODEL", os.environ.get("OLLAMA_MODEL", "mistral-nemo:latest"))

USE_GROQ = bool(GROQ_API_KEY and GROQ_API_KEY != "your_groq_api_key_here")

if USE_GROQ:
    log.info(f"LLM backend: Groq ({GROQ_MODEL})")
else:
    log.info(f"LLM backend: Ollama ({OLLAMA_MODEL} at {OLLAMA_URL})")

MAX_TURNS = 6


# =============================================================================
# LLM call — Groq or Ollama
# =============================================================================
def _chat(messages: list[dict], system: str, tools: list[dict] | None = None, force_tool: bool = False) -> dict:
    """
    Single chat call. Returns the message dict:
      {"role": "assistant", "content": "...", "tool_calls": [...]}
    force_tool: if True, model MUST call a tool (Groq only)
    """
    if USE_GROQ:
        return _chat_groq(messages, system, tools, force_tool=force_tool)
    else:
        return _chat_ollama(messages, system, tools)


def _chat_groq(messages: list[dict], system: str, tools: list[dict] | None = None, force_tool: bool = False) -> dict:
    headers = {
        "Authorization": f"Bearer {GROQ_API_KEY}",
        "Content-Type":  "application/json",
    }
    payload = {
        "model":    GROQ_MODEL,
        "messages": [{"role": "system", "content": system}] + messages,
        "temperature": 0.1,
        "max_tokens":  1024,
    }
    if tools:
        payload["tools"]       = tools
        payload["tool_choice"] = "required" if force_tool else "auto"

    r = requests.post(
        f"{GROQ_BASE_URL}/chat/completions",
        headers=headers,
        json=payload,
        timeout=60,
    )
    r.raise_for_status()
    choice  = r.json()["choices"][0]
    message = choice["message"]
    # Return the raw message dict so tool_calls preserves Groq format
    # (needed when we append it back to messages for multi-turn)
    return {
        "role":            message.get("role", "assistant"),
        "content":         message.get("content") or "",
        "tool_calls":      message.get("tool_calls", []),   # raw Groq format
        "_raw_message":    message,                          # original for re-sending
    }


def _chat_ollama(messages: list[dict], system: str, tools: list[dict] | None = None) -> dict:
    payload = {
        "model":    OLLAMA_MODEL,
        "messages": [{"role": "system", "content": system}] + messages,
        "stream":   False,
        "options":  {"temperature": 0.1, "num_predict": 1024},
    }
    if tools:
        payload["tools"] = tools

    r = requests.post(f"{OLLAMA_URL}/api/chat", json=payload, timeout=180)
    r.raise_for_status()
    message = r.json()["message"]
    return {
        "role":       message.get("role", "assistant"),
        "content":    message.get("content") or "",
        "tool_calls": message.get("tool_calls", []),
    }


# =============================================================================
# Tool schema conversion
# =============================================================================
def _mcp_to_tools(tools: list[dict]) -> list[dict]:
    """Convert MCP tool schema to OpenAI/Groq tool format."""
    result = []
    for t in tools:
        result.append({
            "type": "function",
            "function": {
                "name":        t["name"],
                "description": t.get("description", ""),
                "parameters":  t.get("inputSchema", {"type": "object", "properties": {}}),
            }
        })
    return result


def _parse_tool_calls(message: dict) -> list[dict]:
    """
    Extract tool calls from message. Handles both Groq and Ollama formats.
    Returns list of {"name": str, "arguments": dict}
    Coerces known integer fields (k, n_results, limit) from string to int.
    """
    INTEGER_FIELDS = {"k", "n_results", "limit", "top_k", "max_results"}
    raw = message.get("tool_calls", [])
    calls = []
    for tc in raw:
        if "function" in tc:
            name = tc["function"]["name"]
            args = tc["function"].get("arguments", {})
            if isinstance(args, str):
                try:
                    args = json.loads(args)
                except Exception:
                    args = {}
            # Coerce integer fields — Groq sometimes returns them as strings
            for field in INTEGER_FIELDS:
                if field in args and isinstance(args[field], str):
                    try:
                        args[field] = int(args[field])
                    except ValueError:
                        pass
            calls.append({"name": name, "arguments": args})
    return calls


# =============================================================================
# ReAct agent loop — used by RAG agent
# =============================================================================
async def run_agent_loop(
    question:     str,
    system:       str,
    tools:        list[dict],
    call_tool_fn: Callable[[str, dict], Awaitable[str]],
    on_thinking:  Callable[[str], None] | None = None,
    on_tool_call: Callable[[str, dict, str], None] | None = None,
) -> str:

    llm_tools = _mcp_to_tools(tools)
    messages  = [{"role": "user", "content": question}]

    for turn in range(MAX_TURNS):
        # Force tool use on first turn so model always searches before answering
        force = (turn == 0) and bool(llm_tools)
        message    = _chat(messages, system, llm_tools, force_tool=force)
        tool_calls = _parse_tool_calls(message)
        content    = message.get("content", "") or ""

        if not tool_calls:
            return content

        # Append the raw message back exactly as Groq returned it
        # This preserves the tool_calls format Groq expects in subsequent turns
        raw_msg = message.get("_raw_message") or {
            "role":       "assistant",
            "content":    content or "",
            "tool_calls": message.get("tool_calls", []),
        }
        messages.append(raw_msg)

        if content and on_thinking:
            on_thinking(content)

        # Execute each tool call, matching tool_call_id from Groq response
        raw_tool_calls = message.get("tool_calls", [])
        for i, tc in enumerate(tool_calls):
            name = tc["name"]
            args = tc["arguments"]
            tool_call_id = raw_tool_calls[i].get("id", f"call_{i}") if i < len(raw_tool_calls) else f"call_{i}"

            try:
                result = await call_tool_fn(name, args)
            except Exception as e:
                result = f"Tool error: {e}"
                log.error(f"Tool {name} failed: {e}")

            if on_tool_call:
                on_tool_call(name, args, result)

            messages.append({
                "role":         "tool",
                "tool_call_id": tool_call_id,
                "content":      result,
            })

    messages.append({"role": "user", "content": "Provide your final answer now."})
    message = _chat(messages, system, None)
    return message.get("content", "")


# =============================================================================
# Single-pass synthesis — used by TekmerDB agent
# =============================================================================
def synthesize_with_reliability(
    question:   str,
    raw_answer: str,
    rd:         dict,
    system:     str,
) -> str:
    """
    Single LLM call that produces a structured answer integrating reliability data.

    Output format:
      1. ASSESSMENT — one line verdict
      2. Body — the actual answer
      3. DATA PROVENANCE — compact metrics block

    rd (reliability data) dict keys:
      avg_conf, min_conf, conf_label, verdict,
      total_corr, total_conf, unverified, n, sources
    """
    if not rd:
        return raw_answer

    conflict_note = (
        f"{rd['total_conf']} contradicting claim(s) detected in the knowledge base."
        if rd["total_conf"] > 0
        else "No contradictions detected."
    )

    verification_note = (
        f"None of the {rd['n']} facts have been independently corroborated (single source only)."
        if rd["unverified"] == rd["n"]
        else f"{rd['n'] - rd['unverified']} of {rd['n']} facts independently corroborated."
    )

    sources_str = ", ".join(rd["sources"][:5])
    if len(rd["sources"]) > 5:
        sources_str += f" (+{len(rd['sources'])-5} more)"

    filled = round(rd["avg_conf"] * 10)
    bar    = "#" * filled + "." * (10 - filled)

    provenance_block = (
        f"--- DATA PROVENANCE ---\n"
        f"Confidence: [{bar}] {rd['avg_conf']:.2f} ({rd['conf_label']}) | "
        f"Facts used: {rd['n']} | Conflicts: {rd['total_conf']} | "
        f"Corroborations: {rd['total_corr']}\n"
        f"Sources: {sources_str}"
    )

    synthesis_system = (
        "You are a senior energy compliance analyst writing a briefing for "
        "a C-level executive or regulatory officer. Be direct, concise, and actionable. "
        "Never mention TekmerDB, PFOs, or internal system details."
    )

    prompt = f"""Write a professional compliance briefing using EXACTLY this three-part structure:

**ASSESSMENT:** <one line — verdict based on confidence and conflicts>

<Body — 3-5 sentences answering the question, naturally integrating the confidence level.
If conflicts exist present both sides. If data is insufficient say so.>

**ACTION:** <one concrete sentence — what the reader should do next>

Use this information:

QUESTION: {question}

ANALYSIS FROM KNOWLEDGE BASE:
{raw_answer}

RELIABILITY METRICS:
- Overall confidence: {rd['avg_conf']:.2f} ({rd['conf_label']})
- Verdict: {rd['verdict']}
- Conflicts: {conflict_note}
- Verification: {verification_note}
- Sources: {sources_str}

Write the three-part briefing now. Do not add any text before ASSESSMENT or after ACTION."""

    try:
        message = _chat(
            messages=[{"role": "user", "content": prompt}],
            system=synthesis_system,
            tools=None,
        )
        answer = message.get("content", "").strip()
        # Append provenance block — always, regardless of model output
        return answer + "\n\n" + provenance_block
    except Exception as e:
        log.error(f"Synthesis failed: {e} -- using raw answer")
        return raw_answer + "\n\n" + provenance_block