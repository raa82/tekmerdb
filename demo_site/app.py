"""TekmerDB public demo backend.

Two ways to feed the shared demo engine:
  - POST /api/demo/upload  — a small PDF or pasted text, auto-split into claims
                              via the `tekmerdb-ingest` CLI (subprocess).
  - POST /api/demo/insert  — a single hand-crafted claim (text/confidence/source),
                              proxied straight to the engine's POST /pfo, for
                              deliberately testing contradiction/corroboration.

The engine itself is never exposed publicly — this app is the only thing
Caddy proxies to, and it talks to the engine over 127.0.0.1 only.
"""
import json
import os
import subprocess
import tempfile
import threading
import uuid

import requests
from flask import Flask, jsonify, request, send_from_directory
from flask_limiter import Limiter
from flask_limiter.util import get_remote_address

ENGINE_URL = os.environ.get("TEKMERDB_ENGINE_URL", "http://127.0.0.1:3000")
INGEST_BIN = os.environ.get("TEKMERDB_INGEST_BIN", "/opt/tekmerdb/tekmerdb-ingest")
SCRATCH_DIR = os.environ.get("TEKMERDB_DEMO_SCRATCH", "/tmp/tekmerdb-demo")
MAX_PDF_BYTES = 2 * 1024 * 1024  # 2MB
MAX_TEXT_CHARS = 20_000
MAX_CLAIM_CHARS = 2_000
MAX_SOURCE_CHARS = 200
INGEST_TIMEOUT_SECS = 45

os.makedirs(SCRATCH_DIR, exist_ok=True)

app = Flask(__name__, static_folder="static", static_url_path="")
app.config["MAX_CONTENT_LENGTH"] = MAX_PDF_BYTES + 64 * 1024  # small margin for form overhead

limiter = Limiter(get_remote_address, app=app, storage_uri="memory://")

# Caps how many ingest subprocesses (PDF/text -> claims) run at once. The engine
# itself already serializes embedding/NLI inference behind a single mutex, so this
# is about bounding child-process/log-file sprawl and keeping latency predictable,
# not about avoiding OOM.
_ingest_slots = threading.Semaphore(2)


def error(message, status=400):
    return jsonify({"error": message}), status


@app.get("/")
def index():
    return send_from_directory(app.static_folder, "index.html")


@app.get("/api/demo/status")
def demo_status():
    # The deployed engine release predates the /status rename (see project notes) —
    # it still answers on /health. This proxy hides that detail from the frontend.
    try:
        resp = requests.get(f"{ENGINE_URL}/health", timeout=5)
        resp.raise_for_status()
        return jsonify(resp.json())
    except requests.RequestException:
        return error("demo engine is unreachable (may be mid-reset, try again shortly)", 503)


@app.post("/api/demo/upload")
@limiter.limit("8 per hour")
def upload():
    pdf_file = request.files.get("pdf")
    text = (request.form.get("text") or "").strip()

    if pdf_file and text:
        return error("submit either a PDF or pasted text, not both")
    if not pdf_file and not text:
        return error("submit a PDF file or some pasted text")
    if text and len(text) > MAX_TEXT_CHARS:
        return error(f"pasted text too long (max {MAX_TEXT_CHARS} characters)")

    job_id = uuid.uuid4().hex[:12]
    source_name = f"demo-{job_id}"

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
                "--engine", ENGINE_URL,
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
        pfo = _fetch_pfo(ev.get("pfo_id"))
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
            f"{ENGINE_URL}/pfo",
            json={"claim_text": claim_text, "confidence": confidence, "source": source},
            timeout=15,
        )
    except requests.RequestException:
        return error("demo engine is unreachable (may be mid-reset, try again shortly)", 503)

    if resp.status_code == 422:
        return error(resp.json().get("message") or "insert rejected", 422)
    if not resp.ok:
        return error("demo engine rejected the request", 502)

    body = resp.json()
    if body.get("status") != "inserted" or not body.get("id"):
        return jsonify({"status": body.get("status"), "message": body.get("message")})

    pfo = _fetch_pfo(body["id"])
    if not pfo:
        return jsonify({"status": "inserted", "confidence": body.get("confidence")})
    return jsonify(_render_pfo(pfo, status="inserted"))


def _fetch_pfo(pfo_id):
    if not pfo_id:
        return None
    try:
        resp = requests.get(f"{ENGINE_URL}/pfo/{pfo_id}", timeout=10)
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
