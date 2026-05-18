# TekmerDB

**The database that knows when it's wrong.**

TekmerDB gives AI agents reliable memory — storing not just facts, but how confident to be in each one, where each came from, and when two sources disagree. Unlike a vector database that retrieves everything with equal confidence, TekmerDB detects contradictions mechanically, tracks source reliability over time, and tells you what it doesn't know.

> *Tekmer* comes from the Turkish word for singular, unique, one-of-a-kind — one storage layer that does what no other database does: reason about the reliability of what it holds.

⚠️ **MVP — Pre-release software.** TekmerDB is under active development. APIs may change and it has not been audited for production use.

---

## Who this is for

**AI engineers and developers** — drop TekmerDB behind your RAG pipeline. Your agent gets a memory layer that knows when two sources disagree instead of returning both with equal confidence.

**Tech leads and architects** — evaluating reliability infrastructure for AI systems? Start with [The Fundamental Design Philosophy](https://github.com/raa82/tekmerdb/wiki#the-fundamental-design-philosophy) and [Why TekmerDB Is Not A Truth Machine](https://github.com/raa82/tekmerdb/wiki#why-tekmerdb-is-not-a-truth-machine).

**Business and compliance** — deploying AI in a regulated industry (energy, healthcare, legal, finance)? TekmerDB is the audit trail and provenance layer that makes AI decisions traceable. Read [Why Conflicts Are More Valuable Than Clean Answers](https://github.com/raa82/tekmerdb/wiki#why-conflicts-are-more-valuable-than-clean-answers).

---

## See it in 30 seconds

Insert two contradicting facts. Watch what happens.

```bash
# Fact from a trusted source
curl -s -X POST http://localhost:3000/pfo \
  -H "Content-Type: application/json" \
  -d '{
    "claim_text": "Global coal demand will fall 20% by 2035",
    "confidence": 0.8,
    "source": "IEA World Energy Outlook",
    "domain": "CriticalInfrastructure"
  }'

# Contradicting claim from a lobby group
curl -s -X POST http://localhost:3000/pfo \
  -H "Content-Type: application/json" \
  -d '{
    "claim_text": "Global coal demand will increase 40% by 2035",
    "confidence": 0.8,
    "source": "CoalIndustryLobby2024",
    "domain": "CriticalInfrastructure"
  }'
```

**Both facts flagged. Both confidences reduced. Conflict preserved. Source named.**

```json
{
  "id": "a1b2c3...",
  "confidence": 0.52,
  "conflict_refs": ["e5f6g7..."],
  "corroboration_count": 0,
  "source": "CoalIndustryLobby2024"
}
```

A vector database returns both claims with identical authority. TekmerDB flags the conflict, names the source, reduces confidence on both, and preserves the full provenance chain. No hallucination. No silent resolution.

[**Check the wiki to get more details  about TekmerDB** ](https://github.com/raa82/tekmerdb/wiki)
---

## What it is. What it isn't.

| TekmerDB is | TekmerDB is not |
|---|---|
| A reliability layer for AI agent memory | A general-purpose database |
| A mechanical engine for epistemic trust | A truth machine |
| An audit trail for AI decisions | A replacement for your application logic |
| EU AI Act compliance infrastructure | A vector database with extra features |
| Air-gapped, no cloud, no API keys | A hosted SaaS |

---

## How is it different from a vector database?

A vector database finds similar things fast. It has no concept of whether those things are reliable.

| | Vector DB (RAG) | TekmerDB |
|---|---|---|
| Storage unit | Text chunk | Probabilistic Fact Object (PFO) |
| Confidence | None — all results equal | Mechanically computed 0.0–1.0 |
| Contradiction detection | None | NLI classifier on every insert |
| Source tracking | Filename + page | UUID, weight, full corroboration history |
| Poisoned data | Returned as fact | Flagged, source degraded, conflict named |
| Audit trail | None | Full provenance to EU AI Act standard |
| Deployment | Cloud-dependent | Single air-gapped binary |

**TekmerDB is additive, not a replacement.** Pipe the outputs of your existing RAG pipeline into TekmerDB and your agent gains a memory layer that knows what to trust.

---

## Benchmark: TekmerDB vs RAG

9 compliance questions. Same LLM. Real 510-page IEA document. Three industry sources.

| Test | Question | Winner |
|---|---|---|
| 1 | Global energy demand by 2035 | TekmerDB — 7 conflicts flagged, RAG blended contradictions silently |
| 2 | 1.5°C climate target | TekmerDB — contradictions detected, confidence 0.72 |
| 3 | EU AI Act certification | TekmerDB — clear NO with reasons, RAG returned irrelevant data |
| 4 | **Poisoned data (lobby claim)** | **TekmerDB** — fake claim flagged and source named. RAG opened with it as fact. |
| 5 | Source audit trail | Tie |
| 6 | Regulatory submission decision | TekmerDB — compliance verdict with confidence score and source trace |
| 7 | 2024 actual energy demand | TekmerDB — correct source retrieved, RAG returned projections |
| 8 | Oil demand 2035 and 2050 | TekmerDB — 2 conflicts flagged correctly |
| 9 | Fastest growing energy sources | Tie |

**Final score: TekmerDB 7 — RAG 0 — Ties 2**

[Read the full comparison report →](https://github.com/raa82/tekmerdb/wiki/Benchmark:-TekmerDB-vs-RAG)

---

## Install

**Prerequisites:** Linux x86_64, `wget`

```bash
# 1. clone the repo
git clone https://github.com/raa82/tekmerdb
cd tekmerdb

# 2. run the installer
sudo ./install.sh
```

The installer will:
- Download the pre-built binaries from the GitHub release
- Download the ML models (~420 MB total) from HuggingFace
- Install everything to `/opt/tekmerdb`
- Copy the default config file

**Start the engine:**
```bash
cd /opt/tekmerdb && ./tekmerdb
```

The engine listens on `http://127.0.0.1:3000` by default.

**Configure:**
```bash
nano /opt/tekmerdb/tekmerdb-server.conf
```

---

## Quick start

```bash
# insert a fact
curl -X POST http://localhost:3000/pfo \
  -H "Content-Type: application/json" \
  -d '{
    "claim_text": "North Sea wind capacity reached 35 GW in 2024",
    "confidence": 0.8,
    "source": "IEA Energy Report",
    "domain": "CriticalInfrastructure"
  }'

# semantic search
curl "http://localhost:3000/search?q=North+Sea+wind+capacity&k=5"

# check source reliability
curl "http://localhost:3000/source?name=IEA+Energy+Report"
```

---

## MCP — connect any AI agent

TekmerDB ships with a Model Context Protocol server. Connect any MCP-compatible agent directly to the engine.

**Claude Desktop config:**

```json
{
  "mcpServers": {
    "tekmerdb": {
      "command": "/opt/tekmerdb/tekmerdb-mcp"
    }
  }
}
```

The agent can insert facts, search, check source reliability, and update confidence — all through natural language. No API key. No cloud.

---

## Learn more

- [**Why Traditional RAG Fails**](https://github.com/raa82/tekmerdb/wiki#why-traditional-rag-fails) — the core problem TekmerDB solves
- [**The Confidence Model**](https://github.com/raa82/tekmerdb/wiki#the-confidence-model) — how confidence is computed mechanically
- [**Conflict Detection**](https://github.com/raa82/tekmerdb/wiki#conflict-detection--why-contradictions-are-preserved) — why contradictions are preserved, not resolved
- [**The Epistemic Model**](https://github.com/raa82/tekmerdb/wiki#the-epistemic-model-behind-tekmerdb) — the philosophy behind the architecture
- [**Full Wiki**](https://github.com/raa82/tekmerdb/wiki) — complete documentation

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

Enterprise features (audit log exports, RBAC, EU AI Act compliance reporting, managed hosting) are available under a commercial licence. [Contact us](mailto:tekmerdb@yourdomain.com).
