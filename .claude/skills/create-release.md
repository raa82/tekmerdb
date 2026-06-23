---
name: create-release
description: Bump version, build all binaries, package tarball, write changelog from git history, tag, push, and publish a GitHub release. Use when the user wants to cut a new release of tekmerdb.
tools: Bash, Read, Edit
---

# /create-release — Publish a new TekmerDB release

Automates the full release pipeline: version bump → build → package → changelog → tag → push → GitHub release.

Arguments passed: `$ARGUMENTS`

---

## Step 1 — Determine the new version

Read the current version from `Cargo.toml`:

```bash
grep '^version' Cargo.toml | head -1 | sed -E 's/version = "(.*)"/\1/' | tr -d ' '
```

If `$ARGUMENTS` contains an explicit version string (e.g. `0.2.0`), use that.
Otherwise increment the **patch** component of the current version by 1
(e.g. `0.1.0` → `0.1.1`).

Show the user: "Ready to release **vX.Y.Z**. Proceed?" — wait for confirmation
before doing anything else.

---

## Step 2 — Generate changelog

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

Group the commit lines into sections based on their prefix. Use this mapping:

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

Within each section, list each commit as `- <message>` (strip the hash prefix).
Omit the hash. Omit any "Bump version to" commits.

Show the user the draft changelog and ask: "Does this changelog look right, or
do you want to edit it?" If they say to edit, apply their changes before
continuing.

---

## Step 3 — Build release binaries

```bash
cargo build --release 2>&1
```

If the build fails, **stop here**. Report the error. Do not proceed to tag or push.

Confirm all four binaries exist:

```
target/release/tekmerdb
target/release/tekmerdb-mcp
target/release/tekmerdb-cron
target/release/tekmerdb-ingest
```

---

## Step 4 — Update version in Cargo.toml

Edit the first `version = "..."` line inside the `[package]` section of
`Cargo.toml` to the new version. Do not touch any dependency version lines.

Then run:

```bash
cargo build 2>&1 | tail -3
```

This syncs `Cargo.lock` without rebuilding. If it fails, stop and report.

---

## Step 5 — Package the tarball

Detect architecture:

```bash
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)  ARCH_LABEL="linux-x64" ;;
  aarch64) ARCH_LABEL="linux-arm64" ;;
  *)       echo "Unsupported: $ARCH"; exit 1 ;;
esac
```

Create the tarball in `/tmp/`:

```bash
TARBALL="tekmerdb-v{VERSION}-${ARCH_LABEL}.tar.gz"
tar -czf "/tmp/${TARBALL}" \
  -C target/release \
  tekmerdb tekmerdb-mcp tekmerdb-cron tekmerdb-ingest
```

The tarball must be **flat** (no subdirectory) — the install script's `find`
command expects binaries directly inside the archive root.

Confirm the tarball was created and list its contents:

```bash
tar -tzf "/tmp/${TARBALL}"
```

---

## Step 6 — Commit and tag

```bash
git add Cargo.toml Cargo.lock
git commit -m "Bump version to {VERSION}

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>"
git tag "v{VERSION}"
```

---

## Step 7 — Push

```bash
git push origin main
git push origin "v{VERSION}"
```

If push fails (no remote access, no credentials), stop here. Tell the user:

> "The commit and tag are ready locally. To finish the release manually:
> ```
> git push origin main && git push origin v{VERSION}
> ```
> Then upload `/tmp/{TARBALL}` to the GitHub release."

---

## Step 8 — Create the GitHub release

**Prefer `gh` if available:**

```bash
which gh && gh auth status
```

If `gh` is available and authenticated:

```bash
gh release create "v{VERSION}" "/tmp/${TARBALL}" \
  --repo raa82/tekmerdb \
  --title "v{VERSION}" \
  --notes "{CHANGELOG_BODY}"
```

**Otherwise, use the GitHub API via `curl`:**

Check for a token:

```bash
echo "${GITHUB_TOKEN:-NOT_SET}"
```

If `GITHUB_TOKEN` is not set, stop and tell the user:

> "Set `GITHUB_TOKEN` in your environment and re-run, or use:
> ```
> gh release create v{VERSION} /tmp/{TARBALL} --repo raa82/tekmerdb --title v{VERSION}
> ```"

If the token is available, create the release:

```bash
RESPONSE=$(curl -s -X POST \
  -H "Authorization: Bearer ${GITHUB_TOKEN}" \
  -H "Accept: application/vnd.github+json" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  https://api.github.com/repos/raa82/tekmerdb/releases \
  -d "{
    \"tag_name\": \"v{VERSION}\",
    \"name\": \"v{VERSION}\",
    \"body\": $(echo "{CHANGELOG_BODY}" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))'),
    \"draft\": false,
    \"prerelease\": false
  }")

UPLOAD_URL=$(echo "$RESPONSE" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['upload_url'])" | sed 's/{?name,label}//')
RELEASE_URL=$(echo "$RESPONSE" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d['html_url'])")
```

Then upload the tarball asset:

```bash
curl -s -X POST \
  -H "Authorization: Bearer ${GITHUB_TOKEN}" \
  -H "Content-Type: application/gzip" \
  --data-binary "@/tmp/${TARBALL}" \
  "${UPLOAD_URL}?name=${TARBALL}"
```

---

## Step 9 — Report

Print a clean summary:

```
Released: v{VERSION}
Tag    : https://github.com/raa82/tekmerdb/releases/tag/v{VERSION}
Asset  : {TARBALL}  (tekmerdb, tekmerdb-mcp, tekmerdb-cron, tekmerdb-ingest)
```

Clean up the local tarball:

```bash
rm "/tmp/${TARBALL}"
```
