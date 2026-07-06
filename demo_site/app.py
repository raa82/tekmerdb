"""TekmerDB public demo backend.

Each visitor picks a domain (EU AI Act risk category) and launches an
instance scoped to it -- a docker container running the engine + MCP
server (see docker/Dockerfile, docker/run.sh), auto-removed 60 minutes
after creation regardless of activity. Up to 3 domains can run at once
(fixed host port slots below); a 4th request gets a 409 listing which
domains are currently active instead of evicting anyone.

  - POST /api/demo/launch          — get-or-create a domain's container
  - GET  /api/demo/launch/status   — poll while it warms up
  - GET  /api/demo/status          — engine status + remaining TTL for a domain
  - POST /api/demo/upload          — a small PDF or pasted text, auto-split into
                                      claims via the `tekmerdb-ingest` CLI
  - POST /api/demo/insert          — a single hand-crafted claim, proxied to
                                      the domain's engine POST /pfo
  - GET  /api/demo/search          — proxied to the domain's engine GET /search
  - GET  /api/demo/sources         — proxied to the domain's engine GET /source/all

Every one of the endpoints above except /launch takes a `domain` query
param and resolves which container to talk to per-request straight from
`docker inspect` -- there's no separate state store to drift out of sync
with the containers actually running.

No engine is ever exposed publicly — this app is the only thing Caddy
proxies to, and it talks to engines over 127.0.0.1 only (published from
their containers via 127.0.0.1-only port mappings, see docker/run.sh).
"""
import json
import os
import re
import subprocess
import threading
import time
import uuid
from datetime import datetime, timezone

import requests
from flask import Flask, jsonify, request, send_from_directory
from flask_limiter import Limiter
from flask_limiter.util import get_remote_address

INGEST_BIN = os.environ.get("TEKMERDB_INGEST_BIN", "/opt/tekmerdb/tekmerdb-ingest")
SCRATCH_DIR = os.environ.get("TEKMERDB_DEMO_SCRATCH", "/tmp/tekmerdb-demo")
DOCKER_BIN = os.environ.get("TEKMERDB_DOCKER_BIN", "docker")
RUN_SCRIPT = os.environ.get("TEKMERDB_RUN_SCRIPT", "/opt/tekmerdb-docker/run.sh")
MAX_PDF_BYTES = 2 * 1024 * 1024  # 2MB
MAX_TEXT_CHARS = 20_000
MAX_CLAIM_CHARS = 2_000
MAX_SOURCE_CHARS = 200
INGEST_TIMEOUT_SECS = 45

CONTAINER_PREFIX = "tekmerdb-"
CONTAINER_TTL_SECS = int(os.environ.get("TEKMERDB_CONTAINER_TTL_SECS", "3600"))
# Fixed (engine, mcp) host port pairs -- one per concurrent instance slot.
# Sized for this box's capacity (see docker/run.sh for the per-container
# memory/cpu math): 3 slots.
SLOTS = [(3100, 3101), (3200, 3201), (3300, 3301)]
DOMAIN_RE = re.compile(r"^[A-Za-z]{1,40}$")

os.makedirs(SCRATCH_DIR, exist_ok=True)

app = Flask(__name__, static_folder="static", static_url_path="")
app.config["MAX_CONTENT_LENGTH"] = MAX_PDF_BYTES + 64 * 1024  # small margin for form overhead

limiter = Limiter(get_remote_address, app=app, storage_uri="memory://")

# Caps how many ingest subprocesses (PDF/text -> claims) run at once. The engine
# itself already serializes embedding/NLI inference behind a single mutex, so this
# is about bounding child-process/log-file sprawl and keeping latency predictable,
# not about avoiding OOM.
_ingest_slots = threading.Semaphore(2)

# Guards read-then-launch races between concurrent requests picking the same
# free slot. gunicorn runs this app with a single worker (see
# demo-site-web.service), so a plain in-process lock is enough -- no need for
# a shared external lock.
_containers_lock = threading.Lock()


def error(message, status=400):
    return jsonify({"error": message}), status


@app.get("/")
def index():
    return send_from_directory(app.static_folder, "index.html")


# ── container orchestration ───────────────────────────────────────────────

def _parse_inspect(data):
    env = data.get("Config", {}).get("Env") or []
    domain = next((e[len("DOMAIN="):] for e in env if e.startswith("DOMAIN=")), None)
    if not domain:
        return None

    ports = data.get("NetworkSettings", {}).get("Ports") or {}
    engine_binding = (ports.get("3000/tcp") or [None])[0]
    mcp_binding = (ports.get("3001/tcp") or [None])[0]
    if not engine_binding or not mcp_binding:
        return None

    try:
        created = datetime.strptime(data["Created"][:19], "%Y-%m-%dT%H:%M:%S").replace(tzinfo=timezone.utc)
        age = (datetime.now(timezone.utc) - created).total_seconds()
    except (KeyError, ValueError):
        age = 0.0

    state = data.get("State", {}) or {}
    health = (state.get("Health") or {}).get("Status", "none")

    return {
        "name": data.get("Name", "").lstrip("/"),
        "domain": domain,
        "engine_port": int(engine_binding["HostPort"]),
        "mcp_port": int(mcp_binding["HostPort"]),
        "age": age,
        "health": health,
        "status": state.get("Status"),
    }


def _list_containers():
    names_out = subprocess.run(
        [DOCKER_BIN, "ps", "-a", "--filter", f"name=^{CONTAINER_PREFIX}", "--format", "{{.Names}}"],
        capture_output=True, text=True, timeout=10,
    )
    names = [n for n in names_out.stdout.split() if n]
    if not names:
        return []
    try:
        out = subprocess.run([DOCKER_BIN, "inspect", *names], capture_output=True, text=True, timeout=10)
        data_list = json.loads(out.stdout) if out.stdout else []
    except (subprocess.SubprocessError, ValueError):
        return []

    containers = []
    for data in data_list:
        c = _parse_inspect(data)
        if c:
            containers.append(c)
    return containers


def _should_reap(c):
    return c["age"] >= CONTAINER_TTL_SECS or c["status"] in ("exited", "dead")


def _live_containers():
    """Current containers, reaping (and excluding) any that are expired or crashed."""
    with _containers_lock:
        containers = _list_containers()
        for c in containers:
            if _should_reap(c):
                subprocess.run([DOCKER_BIN, "rm", "-f", c["name"]], capture_output=True, timeout=15)
        return [c for c in containers if not _should_reap(c)]


def _reaper_loop():
    while True:
        try:
            _live_containers()
        except Exception:
            pass
        time.sleep(30)


threading.Thread(target=_reaper_loop, daemon=True).start()


def find_or_launch(domain):
    with _containers_lock:
        containers = _list_containers()
        for c in containers:
            if _should_reap(c):
                subprocess.run([DOCKER_BIN, "rm", "-f", c["name"]], capture_output=True, timeout=15)
        live = [c for c in containers if not _should_reap(c)]

        existing = next((c for c in live if c["domain"].lower() == domain.lower()), None)
        if existing:
            return {
                "domain": existing["domain"],
                "state": "ready" if existing["health"] == "healthy" else "launching",
                "mcp_port": existing["mcp_port"],
                "ttl_remaining": max(0, int(CONTAINER_TTL_SECS - existing["age"])),
            }, 200

        if len(live) >= len(SLOTS):
            active = sorted({c["domain"] for c in live})
            return {"error": "all instance slots are in use", "active_domains": active}, 409

        used_ports = {c["engine_port"] for c in live}
        engine_port, mcp_port = next(s for s in SLOTS if s[0] not in used_ports)

        try:
            subprocess.run(
                [RUN_SCRIPT, domain, str(engine_port), str(mcp_port)],
                check=True, capture_output=True, text=True, timeout=20,
            )
        except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
            return {"error": "failed to launch instance, try again shortly"}, 502

        return {
            "domain": domain,
            "state": "launching",
            "mcp_port": mcp_port,
            "ttl_remaining": CONTAINER_TTL_SECS,
        }, 202


def _resolve_engine_url(domain):
    if not DOMAIN_RE.fullmatch(domain or ""):
        return None
    live = _live_containers()
    c = next((x for x in live if x["domain"].lower() == domain.lower()), None)
    if not c or c["health"] != "healthy":
        return None
    return f"http://127.0.0.1:{c['engine_port']}"


EXPIRED_MSG = "this instance is no longer available — it may have expired, please relaunch"


@app.post("/api/demo/launch")
@limiter.limit("20 per hour")
def launch():
    body = request.get_json(silent=True) or {}
    domain = (body.get("domain") or "").strip()
    if not DOMAIN_RE.fullmatch(domain):
        return error("invalid domain")
    result, status = find_or_launch(domain)
    return jsonify(result), status


@app.get("/api/demo/launch/status")
def launch_status():
    domain = (request.args.get("domain") or "").strip()
    if not DOMAIN_RE.fullmatch(domain):
        return error("invalid domain")

    live = _live_containers()
    c = next((x for x in live if x["domain"].lower() == domain.lower()), None)
    if not c:
        return jsonify({"state": "gone"})

    return jsonify({
        "domain": c["domain"],
        "state": "ready" if c["health"] == "healthy" else "launching",
        "mcp_port": c["mcp_port"],
        "ttl_remaining": max(0, int(CONTAINER_TTL_SECS - c["age"])),
    })


@app.get("/api/demo/status")
def demo_status():
    domain = (request.args.get("domain") or "").strip()
    if not DOMAIN_RE.fullmatch(domain):
        return error("missing or invalid domain")

    live = _live_containers()
    c = next((x for x in live if x["domain"].lower() == domain.lower()), None)
    if not c:
        return error(EXPIRED_MSG, 410)

    try:
        resp = requests.get(f"http://127.0.0.1:{c['engine_port']}/status", timeout=5)
        resp.raise_for_status()
        data = resp.json()
    except requests.RequestException:
        return error("demo engine is unreachable (may still be starting up, try again shortly)", 503)

    return jsonify({
        "status": data.get("status"),
        "domain": data.get("domain"),
        "pfo_count": data.get("pfo_count"),
        "ttl_remaining": max(0, int(CONTAINER_TTL_SECS - c["age"])),
        "mcp_port": c["mcp_port"],
    })


@app.post("/api/demo/upload")
@limiter.limit("8 per hour")
def upload():
    domain = (request.args.get("domain") or "").strip()
    engine_url = _resolve_engine_url(domain)
    if not engine_url:
        return error(EXPIRED_MSG, 410)

    pdf_file = request.files.get("pdf")
    text = (request.form.get("text") or "").strip()
    custom_source = (request.form.get("source") or "").strip()

    if pdf_file and text:
        return error("submit either a PDF or pasted text, not both")
    if not pdf_file and not text:
        return error("submit a PDF file or some pasted text")
    if text and len(text) > MAX_TEXT_CHARS:
        return error(f"pasted text too long (max {MAX_TEXT_CHARS} characters)")
    if custom_source and len(custom_source) > MAX_SOURCE_CHARS:
        return error(f"source name too long (max {MAX_SOURCE_CHARS} characters)")

    job_id = uuid.uuid4().hex[:12]
    source_name = custom_source or f"demo-{job_id}"

    if pdf_file:
        if not pdf_file.filename.lower().endswith(".pdf"):
            return error("uploaded file must be a .pdf")
        tmp_path = os.path.join(SCRATCH_DIR, f"{job_id}.pdf")
        pdf_file.save(tmp_path)
        if os.path.getsize(tmp_path) > MAX_PDF_BYTES:
            os.remove(tmp_path)
            return error(f"PDF too large (max {MAX_PDF_BYTES // 1024 // 1024}MB)")
        doc_type = "pdf"
    else:
        tmp_path = os.path.join(SCRATCH_DIR, f"{job_id}.txt")
        with open(tmp_path, "w", encoding="utf-8") as f:
            f.write(text)
        doc_type = "txt"

    log_path = os.path.join(SCRATCH_DIR, f"{job_id}.ndjson")

    acquired = _ingest_slots.acquire(timeout=5)
    if not acquired:
        _cleanup(tmp_path, log_path)
        return error("demo is busy processing other submissions, try again shortly", 503)

    try:
        result = subprocess.run(
            [
                INGEST_BIN, tmp_path,
                "--engine", engine_url,
                "--source", source_name,
                "--type", doc_type,
                "--log", log_path,
            ],
            capture_output=True,
            text=True,
            timeout=INGEST_TIMEOUT_SECS,
        )
    except subprocess.TimeoutExpired:
        _cleanup(tmp_path, log_path)
        return error("ingest took too long — try a smaller document", 504)
    finally:
        _ingest_slots.release()

    events = _read_ndjson(log_path)
    _cleanup(tmp_path, log_path)

    if result.returncode != 0 and not events:
        return error("ingest failed to reach the demo engine, try again shortly", 502)

    claims = []
    for ev in events:
        if ev.get("result") != "inserted":
            claims.append({
                "claim_text": ev.get("claim"),
                "status": ev.get("result"),
                "reason": ev.get("reason"),
            })
            continue
        pfo = _fetch_pfo(engine_url, ev.get("pfo_id"))
        if pfo:
            claims.append(_render_pfo(pfo, status="inserted"))
        else:
            claims.append({
                "claim_text": ev.get("claim"),
                "status": "inserted",
                "confidence": ev.get("confidence"),
            })

    return jsonify({"source": source_name, "claims": claims})


@app.post("/api/demo/insert")
@limiter.limit("20 per hour")
def insert():
    domain = (request.args.get("domain") or "").strip()
    engine_url = _resolve_engine_url(domain)
    if not engine_url:
        return error(EXPIRED_MSG, 410)

    body = request.get_json(silent=True) or {}
    claim_text = (body.get("claim_text") or "").strip()
    source = (body.get("source") or "").strip()
    confidence = body.get("confidence")

    if not claim_text or len(claim_text) > MAX_CLAIM_CHARS:
        return error(f"claim_text is required (max {MAX_CLAIM_CHARS} characters)")
    if not source or len(source) > MAX_SOURCE_CHARS:
        return error(f"source is required (max {MAX_SOURCE_CHARS} characters)")
    try:
        confidence = float(confidence)
    except (TypeError, ValueError):
        return error("confidence must be a number between 0.0 and 1.0")
    if not (0.0 <= confidence <= 1.0):
        return error("confidence must be between 0.0 and 1.0")

    try:
        resp = requests.post(
            f"{engine_url}/pfo",
            json={"claim_text": claim_text, "confidence": confidence, "source": source},
            timeout=15,
        )
    except requests.RequestException:
        return error("demo engine is unreachable (may still be starting up, try again shortly)", 503)

    if resp.status_code == 422:
        return error(resp.json().get("message") or "insert rejected", 422)
    if not resp.ok:
        return error("demo engine rejected the request", 502)

    body = resp.json()
    if body.get("status") != "inserted" or not body.get("id"):
        return jsonify({"status": body.get("status"), "message": body.get("message")})

    pfo = _fetch_pfo(engine_url, body["id"])
    if not pfo:
        return jsonify({"status": "inserted", "confidence": body.get("confidence")})
    return jsonify(_render_pfo(pfo, status="inserted"))


@app.get("/api/demo/search")
@limiter.limit("30 per hour")
def search():
    domain = (request.args.get("domain") or "").strip()
    engine_url = _resolve_engine_url(domain)
    if not engine_url:
        return error(EXPIRED_MSG, 410)

    q = (request.args.get("q") or "").strip()
    if not q or len(q) > MAX_CLAIM_CHARS:
        return error(f"q is required (max {MAX_CLAIM_CHARS} characters)")

    try:
        resp = requests.get(f"{engine_url}/search", params={"q": q}, timeout=15)
    except requests.RequestException:
        return error("demo engine is unreachable (may still be starting up, try again shortly)", 503)

    if not resp.ok:
        return error("search failed", 502)

    return jsonify(resp.json())


@app.get("/api/demo/sources")
@limiter.limit("30 per hour")
def sources():
    domain = (request.args.get("domain") or "").strip()
    engine_url = _resolve_engine_url(domain)
    if not engine_url:
        return error(EXPIRED_MSG, 410)

    try:
        resp = requests.get(f"{engine_url}/source/all", timeout=15)
    except requests.RequestException:
        return error("demo engine is unreachable (may still be starting up, try again shortly)", 503)

    if not resp.ok:
        return error("failed to fetch sources", 502)

    return jsonify(resp.json())


def _fetch_pfo(engine_url, pfo_id):
    if not pfo_id:
        return None
    try:
        resp = requests.get(f"{engine_url}/pfo/{pfo_id}", timeout=10)
        if resp.ok:
            return resp.json()
    except requests.RequestException:
        pass
    return None


def _render_pfo(pfo, status):
    return {
        "id": pfo.get("id"),
        "claim_text": pfo.get("claim_text"),
        "status": status,
        "confidence": pfo.get("confidence"),
        "source": pfo.get("source"),
        "corroboration_count": pfo.get("corroboration_count", 0),
        "conflict_count": len(pfo.get("conflict_refs") or []),
    }


def _read_ndjson(path):
    events = []
    if not os.path.exists(path):
        return events
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return events


def _cleanup(*paths):
    for p in paths:
        try:
            os.remove(p)
        except OSError:
            pass


if __name__ == "__main__":
    app.run(host="127.0.0.1", port=8090)
