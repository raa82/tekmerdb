// TODO: fetch this from a backend endpoint (mirrors the engine's Domain enum,
// src/engine/pfo.rs). Hardcoded here for now.
const DOMAINS = [
  "Healthcare", "LawEnforcement", "CriticalInfrastructure", "Employment",
  "Education", "Migration", "LegalInterpretation", "Finance", "General",
];

function humanizeDomain(name) {
  return name.replace(/([a-z])([A-Z])/g, "$1 $2");
}

const domainSelect = document.getElementById("domain-select");
DOMAINS.forEach((d) => {
  const opt = document.createElement("option");
  opt.value = d;
  opt.textContent = humanizeDomain(d);
  domainSelect.appendChild(opt);
});

document.querySelectorAll(".side-tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".side-tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".side-tab-panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById(`side-tab-${btn.dataset.sideTab}`).classList.add("active");
  });
});

// ── launch flow ────────────────────────────────────────────────────────────

const launchScreen = document.getElementById("launch-screen");
const prepScreen = document.getElementById("prep-screen");
const appScreen = document.getElementById("app-screen");
const launchErrorEl = document.getElementById("launch-error");
const prepMessageEl = document.getElementById("prep-message");
const mcpEndpointEl = document.getElementById("mcp-endpoint");
const sysExpiredNoteEl = document.getElementById("sys-expired-note");

const PREP_MESSAGES = [
  "Preparing environment…",
  "Setting up instance…",
  "Loading domain models…",
  "Almost ready…",
];

let currentDomain = sessionStorage.getItem("tekmerdb_domain") || null;
let prepMsgTimer = null;
let launchPollTimer = null;
let ttlDeadline = null;

function showLaunchScreen(errMsg) {
  clearInterval(prepMsgTimer);
  clearTimeout(launchPollTimer);
  currentDomain = null;
  ttlDeadline = null;
  sessionStorage.removeItem("tekmerdb_domain");
  prepScreen.hidden = true;
  appScreen.hidden = true;
  launchScreen.hidden = false;
  if (errMsg) {
    launchErrorEl.textContent = errMsg;
    launchErrorEl.hidden = false;
  } else {
    launchErrorEl.hidden = true;
  }
}

function showPrepScreen() {
  launchScreen.hidden = true;
  appScreen.hidden = true;
  prepScreen.hidden = false;
  let i = 0;
  prepMessageEl.textContent = PREP_MESSAGES[0];
  clearInterval(prepMsgTimer);
  prepMsgTimer = setInterval(() => {
    i = (i + 1) % PREP_MESSAGES.length;
    prepMessageEl.textContent = PREP_MESSAGES[i];
  }, 2200);
}

function updateMcpEndpoint(port) {
  if (port) mcpEndpointEl.textContent = `http://tekmerdb.com:${port}/sse`;
}

function enterAppScreen(domain, mcpPort) {
  clearInterval(prepMsgTimer);
  clearTimeout(launchPollTimer);
  currentDomain = domain;
  sessionStorage.setItem("tekmerdb_domain", domain);
  sysExpiredNoteEl.hidden = true;
  prepScreen.hidden = true;
  launchScreen.hidden = true;
  appScreen.hidden = false;
  updateMcpEndpoint(mcpPort);
  refreshStatus();
}

async function pollLaunchStatus(domain) {
  try {
    const resp = await fetch(`/api/demo/launch/status?domain=${encodeURIComponent(domain)}`);
    const data = await resp.json();
    if (data.state === "ready") {
      enterAppScreen(domain, data.mcp_port);
      return;
    }
    if (data.state === "gone") {
      showLaunchScreen("Instance failed to start — try again.");
      return;
    }
  } catch (e) {
    // transient — keep polling
  }
  launchPollTimer = setTimeout(() => pollLaunchStatus(domain), 1500);
}

document.getElementById("launch-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const domain = domainSelect.value;
  showPrepScreen();
  try {
    const resp = await fetch("/api/demo/launch", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ domain }),
    });
    const data = await resp.json();
    if (resp.status === 409) {
      const active = (data.active_domains || []).map(humanizeDomain).join(", ") || "none";
      showLaunchScreen(`All 3 instance slots are busy right now (active: ${active}). Try again shortly, or pick one of those domains.`);
      return;
    }
    if (!resp.ok) {
      showLaunchScreen(data.error || "couldn't launch instance");
      return;
    }
    if (data.state === "ready") {
      enterAppScreen(domain, data.mcp_port);
    } else {
      pollLaunchStatus(domain);
    }
  } catch (e) {
    showLaunchScreen("network error — try again");
  }
});

// Resume mid-session across a page refresh, within the same tab.
if (currentDomain) {
  showPrepScreen();
  pollLaunchStatus(currentDomain);
}

// ── sysinfo / status polling ────────────────────────────────────────────────

const consoleEl = document.getElementById("console");
const sysDomainEl = document.getElementById("sys-domain");
const sysPfosEl = document.getElementById("sys-pfos");
const sysTtlEl = document.getElementById("sys-ttl");

function formatCountdown(ms) {
  if (ms <= 0) return "0:00";
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

async function refreshStatus() {
  if (!currentDomain) return;
  try {
    const resp = await fetch(`/api/demo/status?domain=${encodeURIComponent(currentDomain)}`);
    const data = await resp.json();
    if (resp.status === 410) {
      sysExpiredNoteEl.hidden = false;
      showLaunchScreen("Your instance expired after 60 minutes. Pick a domain to launch a new one.");
      return;
    }
    if (!resp.ok) {
      sysDomainEl.textContent = "unreachable";
      sysPfosEl.textContent = "—";
      sysTtlEl.textContent = "—";
      return;
    }
    sysDomainEl.textContent = data.domain || "—";
    sysPfosEl.textContent = typeof data.pfo_count === "number" ? data.pfo_count : "—";
    if (typeof data.ttl_remaining === "number") {
      ttlDeadline = Date.now() + data.ttl_remaining * 1000;
      sysTtlEl.textContent = formatCountdown(ttlDeadline - Date.now());
    }
    updateMcpEndpoint(data.mcp_port);
  } catch (e) {
    sysDomainEl.textContent = "unreachable";
  }
}

setInterval(refreshStatus, 20000);

setInterval(() => {
  if (ttlDeadline == null || appScreen.hidden) return;
  const remaining = ttlDeadline - Date.now();
  sysTtlEl.textContent = formatCountdown(remaining);
  if (remaining <= 0) {
    showLaunchScreen("Your instance expired after 60 minutes. Pick a domain to launch a new one.");
  }
}, 1000);

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (m) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[m]));
}

function scrollToBottom() {
  consoleEl.scrollTop = consoleEl.scrollHeight;
}

function printPrompt(cmd) {
  const el = document.createElement("div");
  el.className = "line";
  el.innerHTML = `<span class="prompt">$</span> ${escapeHtml(cmd)}`;
  consoleEl.appendChild(el);
  scrollToBottom();
}

function printPending(text) {
  const el = document.createElement("div");
  el.className = "line pending";
  el.textContent = text;
  consoleEl.appendChild(el);
  scrollToBottom();
  return el;
}

function claimLineHtml(c) {
  const pct = typeof c.confidence === "number" ? `${Math.round(c.confidence * 100)}%` : null;
  const tags = [];
  if (c.status && c.status !== "inserted") tags.push(`[${c.status}]`);
  if (c.conflict_count) tags.push(`[conflict x${c.conflict_count}]`);
  if (c.corroboration_count) tags.push(`[corroborated x${c.corroboration_count}]`);

  let cls = "out";
  if (c.conflict_count) cls += " conflict";
  else if (c.corroboration_count) cls += " corroborated";
  else if (c.status && c.status !== "inserted") cls += " rejected";

  const parts = [];
  if (pct) parts.push(`<span class="pct">[${pct}]</span>`);
  parts.push(escapeHtml(c.claim_text || ""));
  if (c.reason) parts.push(`&mdash; ${escapeHtml(c.reason)}`);
  if (tags.length) parts.push(`<span class="tag">${escapeHtml(tags.join(" "))}</span>`);

  return `<div class="${cls}">${parts.join(" ")}</div>`;
}

function renderClaims(claims) {
  if (!claims || claims.length === 0) {
    return `<div class="out">no claims were extracted.</div>`;
  }
  return claims.map(claimLineHtml).join("");
}

async function runJob(cmd, request) {
  printPrompt(cmd);
  const pending = printPending("... working (embedding + conflict detection takes a few seconds)");
  try {
    const resp = await fetch(request.url, request.options);
    const data = await resp.json();
    if (resp.status === 410) {
      pending.remove();
      showLaunchScreen("Your instance expired after 60 minutes. Pick a domain to launch a new one.");
      return;
    }
    if (!resp.ok) {
      pending.outerHTML = `<div class="out error">!! ${escapeHtml(data.error || "something went wrong")}</div>`;
      scrollToBottom();
      return;
    }
    const claims = request.extractClaims(data);
    pending.outerHTML = renderClaims(claims);
    scrollToBottom();
    refreshStatus();
  } catch (e) {
    pending.outerHTML = `<div class="out error">!! network error &mdash; try again</div>`;
    scrollToBottom();
  }
}

function setBusy(form, busy) {
  form.querySelector("button").disabled = busy;
}

document.getElementById("pdf-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  const filename = form.pdf.files[0] ? form.pdf.files[0].name : "(no file)";
  setBusy(form, true);
  await runJob(`ingest --pdf ${filename}`, {
    url: `/api/demo/upload?domain=${encodeURIComponent(currentDomain)}`,
    options: { method: "POST", body: new FormData(form) },
    extractClaims: (data) => data.claims,
  });
  setBusy(form, false);
});

document.getElementById("text-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  const source = form.source.value.trim();
  setBusy(form, true);
  await runJob(`ingest --text${source ? ` --source ${source}` : ""}`, {
    url: `/api/demo/upload?domain=${encodeURIComponent(currentDomain)}`,
    options: { method: "POST", body: new FormData(form) },
    extractClaims: (data) => data.claims,
  });
  setBusy(form, false);
});

function renderJsonValue(value, indent) {
  const pad = "  ".repeat(indent);
  const padInner = "  ".repeat(indent + 1);
  if (Array.isArray(value)) {
    if (value.length === 0) return "[]";
    const items = value.map((v) => padInner + renderJsonValue(v, indent + 1)).join(",\n");
    return `[\n${items}\n${pad}]`;
  }
  if (value !== null && typeof value === "object") {
    const keys = Object.keys(value);
    if (keys.length === 0) return "{}";
    const items = keys
      .map((k) => {
        const keyHtml = `<span class="json-key">${escapeHtml(JSON.stringify(k))}</span>`;
        return `${padInner}${keyHtml}: ${renderJsonValue(value[k], indent + 1)}`;
      })
      .join(",\n");
    return `{\n${items}\n${pad}}`;
  }
  return `<span class="json-val">${escapeHtml(JSON.stringify(value))}</span>`;
}

async function runQueryJob(cmd, url) {
  printPrompt(cmd);
  const pending = printPending("... fetching");
  try {
    const resp = await fetch(url);
    const data = await resp.json();
    if (resp.status === 410) {
      pending.remove();
      showLaunchScreen("Your instance expired after 60 minutes. Pick a domain to launch a new one.");
      return;
    }
    if (!resp.ok) {
      pending.outerHTML = `<div class="out error">!! ${escapeHtml(data.error || "something went wrong")}</div>`;
      scrollToBottom();
      return;
    }
    pending.outerHTML = `<pre class="json-output">${renderJsonValue(data, 0)}</pre>`;
    scrollToBottom();
  } catch (e) {
    pending.outerHTML = `<div class="out error">!! network error &mdash; try again</div>`;
    scrollToBottom();
  }
}

document.getElementById("search-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  const q = form.q.value.trim();
  setBusy(form, true);
  await runQueryJob(`search "${q}"`, `/api/demo/search?domain=${encodeURIComponent(currentDomain)}&q=${encodeURIComponent(q)}`);
  setBusy(form, false);
});

document.getElementById("sources-btn").addEventListener("click", async (e) => {
  const btn = e.target;
  btn.disabled = true;
  await runQueryJob("sources --all", `/api/demo/sources?domain=${encodeURIComponent(currentDomain)}`);
  btn.disabled = false;
});

document.getElementById("claim-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const form = e.target;
  const fd = new FormData(form);
  const claimText = fd.get("claim_text");
  const confidence = parseFloat(fd.get("confidence"));
  const source = fd.get("source");
  setBusy(form, true);
  await runJob(`insert --source ${source} --confidence ${confidence} "${claimText}"`, {
    url: `/api/demo/insert?domain=${encodeURIComponent(currentDomain)}`,
    options: {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ claim_text: claimText, confidence, source }),
    },
    extractClaims: (data) => [data],
  });
  setBusy(form, false);
});
