# TekmerDB
<img width="1536" height="1024" alt="9fffbfec-4dbd-441d-ba6c-cebc03dc4152" src="https://github.com/user-attachments/assets/eb918a70-9371-401c-ae11-cf024886c24d" />
> ⚠️ **MVP — Pre-release software.** TekmerDB is under active development. APIs may change, features are incomplete, and it has not been audited for production use. Use at your own risk.

---

**TekmerDB is the database that knows when it's wrong.**

It gives AI agents reliable memory — storing not just facts, but how confident to be in each one, where each came from, and when two sources disagree. Unlike a vector database that retrieves everything with equal confidence, TekmerDB detects contradictions mechanically, tracks source reliability over time, and tells you what it doesn't know.

> **Why TekmerDB?** *Tekmer* comes from the Turkish word for singular, unique, one-of-a-kind. One storage layer that does what no other database does: reason about the reliability of what it holds.

---

## What it is. What it isn't.

| TekmerDB is | TekmerDB is not |
|---|---|
| A storage layer for AI agent memory | A general-purpose database |
| A reliability engine for facts | A truth machine |
| An audit trail for AI decisions | A replacement for your application logic |
| EU AI Act compliance infrastructure | A vector database with extra features |
| Air-gapped, no cloud, no API keys | A hosted SaaS |

---

## How is it different from RAG + Vector DB?

A vector database finds similar things fast. It has no concept of whether those things are reliable.

| | RAG + Vector DB | TekmerDB |
|---|---|---|
| Storage unit | Text chunk | Probabilistic Fact Object (PFO) |
| Confidence | None — all results equal | Mechanically computed 0.0–1.0 |
| Contradiction detection | None | NLI classifier on every insert |
| Source tracking | Filename + page | UUID, weight, corroboration history |
| Poisoned data | Returned as fact | Flagged, source degraded |
| Audit trail | None | Full provenance to EU AI Act standard |
| Replaces | — | RAG pipeline + vector DB |

**The simplest way to see the difference:**

Insert a fact from a trusted source. Then insert a contradicting fact from a lobby group. A vector database returns both with identical confidence. TekmerDB flags the conflict, names the source, reduces confidence on both, and tells you exactly what happened.

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
# edit before starting
nano /opt/tekmerdb/tekmerdb-server.conf
```

---

## Quick start

```bash
# insert a fact
curl -X POST http://localhost:3000/pfo \
  -H "Content-Type: application/json" \
  -d '{"claim_text": "North Sea wind capacity reached 35 GW in 2024",
       "confidence": 0.8,
       "source": "IEA Energy Report",
       "domain": "CriticalInfrastructure"}'

# insert a contradicting fact
curl -X POST http://localhost:3000/pfo \
  -H "Content-Type: application/json" \
  -d '{"claim_text": "North Sea wind capacity remained below 20 GW in 2024",
       "confidence": 0.8,
       "source": "CoalLobby2024",
       "domain": "CriticalInfrastructure"}'

# retrieve — both facts flagged, confidence reduced, conflict refs populated
curl http://localhost:3000/search?q=North+Sea+wind+capacity
```

---

## MCP (AI agent interface)

TekmerDB ships with a Model Context Protocol server. Connect any MCP-compatible AI agent directly to the engine.

**Claude Desktop config**:

```json
{
  "mcpServers": {
    "tekmerdb": {
      "command": "/opt/tekmerdb/tekmerdb-mcp"
    }
  }
}
```

The agent can then insert facts, search, check source reliability, and update confidence — all through natural language.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

Enterprise features (audit log exports, RBAC, EU AI Act compliance reporting, managed hosting) are available under a commercial licence. Contact us.
