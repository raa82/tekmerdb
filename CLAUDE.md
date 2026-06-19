# TekmerDB

Claude responded: TekmerDB is an air-gapped Rust database that gives AI agents reliable memory — it knows not just facts, but how confident to be in each one and when sources di…TekmerDB is an air-gapped Rust database that gives AI agents reliable memory — it knows not just facts, but how confident to be in each one and when sources disagree. Confidence is computed mechanically, not guessed by an LLM, and conflicts are preserved as signal rather than hidden. Built for EU AI Act compliance, starting in the energy sector.

## Hard rules
- No hardcoded word/file lists — config or data-driven only
- Scripts, not interactive sessions
- [any other standing constraints you don't want re-debated]
- You only can build/create new binaries if you have authorization for it. Ask first

## Where things live
- `src/` — see docs/architecture.md for module map
- `docs/decisions.md` — rejected approaches, don't re-propose these
- `/search` API: `q=` param, returns flat JSON array of PFOs

## Build/test
- always offer cargo build after code changes
- `cargo build`, `cargo test` — [any non-default flags/features]
  
## Git workflow
- After completing a full task/request (not after each individual file edit), commit with `git add -A && git commit -m "..."`.
- Write a brief, descriptive commit message summarizing what changed and why.
- Only commit after cargo build
- Never push — commits stay local until pushed manually.