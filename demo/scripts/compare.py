#!/usr/bin/env python3
"""
compare.py — Side-by-side comparison of both agents.

Sends the same question to both agents simultaneously via HTTP:
  Agent A (TekmerDB): http://localhost:8081/ask
  Agent B (RAG):      http://localhost:8080/ask

Both agents must be running before calling this script.

Usage:
    # Interactive — type questions one at a time:
    python scripts/compare.py

    # Single question:
    python scripts/compare.py -q "Can we certify our offshore wind data for EU AI Act?"

    # From a file (one question per line, # for comments):
    python scripts/compare.py -f questions.txt

    # Custom agent URLs:
    python scripts/compare.py \
        --tekmerdb-url http://localhost:8081 \
        --rag-url http://localhost:8080
"""

import argparse
import json
import sys
import textwrap
import time
from datetime import datetime
from pathlib import Path

import requests

TEKMERDB_URL = "http://localhost:8081"
RAG_URL      = "http://localhost:8080"
LOG_DIR      = "./logs"
W            = 76

# =============================================================================
# HTTP helpers
# =============================================================================

def check_agent(url: str, label: str) -> bool:
    try:
        r = requests.get(f"{url}/health", timeout=5)
        data = r.json()
        status = data.get("status", "?")
        print(f"  ✓ {label:<20} {status}  ({url})")
        return status == "ready"
    except Exception as e:
        print(f"  ✗ {label:<20} unreachable — {e}")
        return False


def ask_agent(url: str, question: str, timeout: int = 180) -> dict:
    try:
        r = requests.post(
            f"{url}/ask",
            json    = {"question": question},
            timeout = timeout,
        )
        r.raise_for_status()
        return r.json()
    except Exception as e:
        return {"answer": f"[Agent error: {e}]", "tool_calls": [], "elapsed_s": 0, "backend": "error"}


# =============================================================================
# Display
# =============================================================================

def hr(char="═", width=W):
    print(char * width)

def wrap_block(text: str, width: int) -> list[str]:
    lines = []
    for paragraph in text.splitlines():
        if not paragraph.strip():
            lines.append("")
        else:
            lines.extend(textwrap.wrap(paragraph, width) or [""])
    return lines

def print_side_by_side(left: str, right: str, col_w: int = 36):
    left_lines  = wrap_block(left,  col_w)
    right_lines = wrap_block(right, col_w)
    n           = max(len(left_lines), len(right_lines))
    left_lines  += [""] * (n - len(left_lines))
    right_lines += [""] * (n - len(right_lines))

    border = "│"
    print(f"  {'RAG  (chroma-mcp)':<{col_w}}  {border}  {'TekmerDB  (pfodb-mcp)':<{col_w}}")
    print(f"  {'─'*col_w}  {border}  {'─'*col_w}")
    for l, r in zip(left_lines, right_lines):
        print(f"  {l:<{col_w}}  {border}  {r:<{col_w}}")
    print(f"  {'─'*col_w}  {border}  {'─'*col_w}")


def confidence_bar(conf: float, width: int = 10) -> str:
    filled = round(conf * width)
    return "[" + "█" * filled + "░" * (width - filled) + f"] {conf:.3f}"


def print_tool_summary(label: str, calls: list[dict], backend: str):
    if not calls:
        print(f"    {label}: no tool calls")
        return
    print(f"    {label}:")
    for tc in calls:
        name   = tc["tool"]
        args   = tc["args"]
        result = tc.get("result", "")
        # Try to parse result for summary
        try:
            data  = json.loads(result)
            items = data if isinstance(data, list) else [data]
            count = len(items)

            if backend == "tekmerdb":
                # Show confidence range across returned PFOs
                confs    = [i.get("confidence") for i in items if i.get("confidence") is not None]
                conflicts = sum(1 for i in items if i.get("conflict_refs"))
                conf_str = f"  conf: {min(confs):.2f}–{max(confs):.2f}" if confs else ""
                flag_str = f"  ⚠ {conflicts} conflict(s)" if conflicts else ""
                print(f"      • {name}({json.dumps(args)}) → {count} PFOs{conf_str}{flag_str}")
            else:
                print(f"      • {name}({json.dumps(args)}) → {count} chunk(s)")
        except Exception:
            print(f"      • {name}({json.dumps(args)}) → {result[:60]}")


def print_comparison(question: str, rag: dict, tekmer: dict, idx: int = 1):
    hr("═")
    print(f"  Question {idx}: {question}")
    hr("─")

    print(f"\n  Tool calls:")
    print_tool_summary("RAG     ", rag.get("tool_calls", []),    "rag")
    print_tool_summary("TekmerDB", tekmer.get("tool_calls", []), "tekmerdb")

    print(f"\n  Answers:\n")
    print_side_by_side(rag.get("answer", ""), tekmer.get("answer", ""))

    rag_t    = rag.get("elapsed_s", 0)
    tekmer_t = tekmer.get("elapsed_s", 0)
    print(f"\n  Time:  RAG {rag_t:.1f}s  |  TekmerDB {tekmer_t:.1f}s")

    # Highlight what TekmerDB surfaced that RAG couldn't
    all_tekmer_results = []
    for tc in tekmer.get("tool_calls", []):
        try:
            data = json.loads(tc.get("result", "[]"))
            if isinstance(data, list):
                all_tekmer_results.extend(data)
        except Exception:
            pass

    conflicts_found = [r for r in all_tekmer_results if r.get("conflict_refs")]
    low_conf        = [r for r in all_tekmer_results if r.get("confidence", 1.0) < 0.6]

    if conflicts_found or low_conf:
        print(f"\n  \033[93m⚠  TekmerDB surfaced (RAG cannot see this):\033[0m")
        if conflicts_found:
            print(f"     {len(conflicts_found)} fact(s) with unresolved conflicts")
        if low_conf:
            print(f"     {len(low_conf)} fact(s) with confidence < 0.6")


# =============================================================================
# Main
# =============================================================================

def run(questions: list[str], tekmerdb_url: str, rag_url: str):
    print()
    hr()
    print("  ENERGY COMPLIANCE ANALYST — Side-by-Side Comparison")
    print(f"  TekmerDB : {tekmerdb_url}")
    print(f"  RAG      : {rag_url}")
    hr()

    print("\n  Checking agents...")
    rag_ready    = check_agent(rag_url,      "Agent B (RAG)")
    tekmer_ready = check_agent(tekmerdb_url, "Agent A (TekmerDB)")

    if not (rag_ready and tekmer_ready):
        print("\n  One or more agents not ready. Start them first:")
        print("    Agent B (RAG):      docker compose up -d agent_rag")
        print("    Agent A (TekmerDB): python agent-tekmerdb/agent_tekmerdb.py --mcp-bin ./pfodb-mcp --serve")
        sys.exit(1)

    Path(LOG_DIR).mkdir(exist_ok=True)
    ts       = datetime.now().strftime("%Y%m%d_%H%M%S")
    log_path = Path(LOG_DIR) / f"compare_{ts}.jsonl"

    with open(log_path, "w") as log_fh:
        for i, question in enumerate(questions, 1):
            print(f"\n  [{i}/{len(questions)}] Running agents sequentially...")

            # Sequential with gap to avoid Groq rate limit
            print("  Running TekmerDB...")
            tekmer_result = ask_agent(tekmerdb_url, question)
            time.sleep(2)
            print("  Running RAG...")
            rag_result = ask_agent(rag_url, question)

            print_comparison(question, rag_result, tekmer_result, idx=i)

            log_fh.write(json.dumps({
                "question": question,
                "rag":      rag_result,
                "tekmerdb": tekmer_result,
            }) + "\n")
            log_fh.flush()

    hr()
    print(f"  Comparison complete. Log: {log_path}")
    hr()


def main():
    parser = argparse.ArgumentParser(description="Side-by-side agent comparison")
    parser.add_argument("-q", "--question",      help="Single question")
    parser.add_argument("-f", "--question-file", help="File with one question per line")
    parser.add_argument("--tekmerdb-url", default=TEKMERDB_URL)
    parser.add_argument("--rag-url",      default=RAG_URL)
    args = parser.parse_args()

    if args.question:
        questions = [args.question]
    elif args.question_file:
        with open(args.question_file) as f:
            questions = [l.strip() for l in f if l.strip() and not l.startswith("#")]
    else:
        # Interactive
        print("\n  Enter questions (empty line to run, Ctrl+C to exit):")
        questions = []
        while True:
            try:
                q = input("  > ").strip()
            except (KeyboardInterrupt, EOFError):
                break
            if not q and questions:
                break
            if q:
                questions.append(q)

    if not questions:
        print("  No questions provided.")
        sys.exit(0)

    run(questions, args.tekmerdb_url, args.rag_url)


if __name__ == "__main__":
    main()