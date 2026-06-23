---
name: create-release
description: Bump version, build all binaries, package tarball, write changelog from git history, tag, push, and publish a GitHub release. Use when the user wants to cut a new release of tekmerdb.
user-invocable: true
tools: Bash, Read, Edit
---

# /create-release — Publish a new TekmerDB release

Automates the full release pipeline: version bump → build → package → changelog → tag → push → GitHub release. Runs end-to-end with no manual steps.

Arguments passed: `$ARGUMENTS`

---

## Step 1 — Resolve GitHub token (do this first, fail fast)

Load the token from the environment or the stored token file:

```bash
source ~/.bashrc 2>/dev/null || true
TOKEN="${GITHUB_TOKEN:-}"
if [ -z "$TOKEN" ] && [ -f ~/.config/tekmerdb/github_token ]; then
  TOKEN=$(cat ~/.config/tekmerdb/github_token)
fi
```

If `TOKEN` is still empty, **stop immediately** and tell the user:

> "No GitHub token found. Add one of:
> - `export GITHUB_TOKEN=your_token` to `~/.bashrc`
> - Paste the token into `~/.config/tekmerdb/github_token` (chmod 600)
>
> Token needs: Contents → Read and write (fine-grained, repo: raa82/tekmerdb)"

Do not proceed past this step without a token.

---

## Step 2 — Determine the new version

Read the current version from `Cargo.toml`:

```bash
grep '^version' Cargo.toml | head -1 | sed -E 's/version = "(.*)"/\1/' | tr -d ' '
```

If `$ARGUMENTS` contains an explicit version string (e.g. `0.2.0`), use that.
Otherwise increment the **patch** component by 1 (e.g. `0.1.1` → `0.1.2`).

Show the user: "Ready to release **vX.Y.Z**. Proceed?" and wait for confirmation.

---

## Step 3 — Generate changelog

Find the previous tag:

```bash
git describe --tags --abbrev=0 2>/dev/null || echo "NONE"
```

If a previous tag exists, collect commits since then:

```bash
git log {PREV_TAG}..HEAD --oneline
```

If no previous tag, use all commits:

```bash
git log --oneline
```

Group commit lines into sections by prefix:

| Prefix keywords | Section header |
|---|---|
| `feat`, `add`, `Add`, `new` | **New features** |
| `fix`, `Fix` | **Bug fixes** |
| `install`, `Install` | **Install / packaging** |
| `refactor`, `Refactor` | **Refactoring** |
| `docs`, `Docs` | **Documentation** |
| `cron`, `Cron` | **Cron / scheduler** |
| `ingest`, `Ingest` | **Ingestor** |
| `mcp`, `MCP` | **MCP server** |
| anything else | **Other changes** |

List each commit as `- <message>` (strip the hash). Omit "Bump version to" commits.

Show the draft changelog and ask: "Does this look right?" Apply any edits before continuing.

---

## Step 4 — Build release binaries

```bash
cargo build --release 2>&1
```

If the build fails, **stop**. Do not tag or push.

Confirm all four binaries exist in `target/release/`:
`tekmerdb`, `tekmerdb-mcp`, `tekmerdb-cron`, `tekmerdb-ingest`

---

## Step 5 — Update version in Cargo.toml

Edit the first `version = "..."` line in the `[package]` section to the new version.
Do not touch dependency version lines.

Sync `Cargo.lock`:

```bash
cargo build 2>&1 | tail -3
```

If it fails, stop and report.

---

## Step 6 — Package the tarball

```bash
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)  ARCH_LABEL="linux-x64" ;;
  aarch64) ARCH_LABEL="linux-arm64" ;;
  *)       echo "Unsupported: $ARCH"; exit 1 ;;
esac
TARBALL="tekmerdb-v{VERSION}-${ARCH_LABEL}.tar.gz"
tar -czf "/tmp/${TARBALL}" \
  -C target/release \
  tekmerdb tekmerdb-mcp tekmerdb-cron tekmerdb-ingest
tar -tzf "/tmp/${TARBALL}"
```

Tarball must be **flat** — the install script's `find` expects binaries at the archive root.

---

## Step 7 — Commit and tag

Note: `Cargo.lock` is gitignored in this repo — only commit `Cargo.toml`.

```bash
git add Cargo.toml
git commit -m "Bump version to {VERSION}

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
git tag "{VERSION}"
```

---

## Step 8 — Push

```bash
git push origin main
git push origin "{VERSION}"
```

If push fails, stop and tell the user to push manually, then upload the tarball at `/tmp/{TARBALL}`.

---

## Step 9 — Create the GitHub release (fully automated)

Use the token resolved in Step 1. Build the JSON body using python3 to handle escaping:

```bash
BODY=$(python3 -c "
import json, sys
print(json.dumps('''CHANGELOG_BODY_HERE'''))
")

RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  https://api.github.com/repos/raa82/tekmerdb/releases \
  -d "{
    \"tag_name\": \"{VERSION}\",
    \"name\": \"TekmerDB {VERSION}\",
    \"body\": ${BODY},
    \"draft\": false,
    \"prerelease\": false
  }")

UPLOAD_URL=$(echo "$RESPONSE" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d['upload_url'])" \
  | sed 's/{?name,label}//')
RELEASE_URL=$(echo "$RESPONSE" | python3 -c \
  "import sys,json; d=json.load(sys.stdin); print(d['html_url'])")
```

If `RELEASE_URL` is empty or the response contains `"message"` (API error), stop and print the full response for diagnosis.

Upload the tarball:

```bash
curl -s -X POST \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "Content-Type: application/gzip" \
  --data-binary "@/tmp/${TARBALL}" \
  "${UPLOAD_URL}?name=${TARBALL}"
```

---

## Step 10 — Report and clean up

```bash
rm "/tmp/${TARBALL}"
```

Print:

```
Released : TekmerDB {VERSION}
Tag      : {VERSION}
URL      : https://github.com/raa82/tekmerdb/releases/tag/{VERSION}
Asset    : {TARBALL}
Binaries : tekmerdb  tekmerdb-mcp  tekmerdb-cron  tekmerdb-ingest
```
