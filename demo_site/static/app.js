document.querySelectorAll(".side-tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".side-tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".side-tab-panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById(`side-tab-${btn.dataset.sideTab}`).classList.add("active");
  });
});

const consoleEl = document.getElementById("console");
const sysDomainEl = document.getElementById("sys-domain");
const sysPfosEl = document.getElementById("sys-pfos");
const sysResetEl = document.getElementById("sys-reset");

function relativeTime(epochSeconds) {
  const diff = Math.max(0, Date.now() / 1000 - epochSeconds);
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

async function refreshStatus() {
  try {
    const resp = await fetch("/api/demo/status");
    const data = await resp.json();
    if (!resp.ok) {
      sysDomainEl.textContent = "unreachable";
      sysPfosEl.textContent = "—";
      sysResetEl.textContent = "—";
      return;
    }
    sysDomainEl.textContent = data.domain || "—";
    sysPfosEl.textContent = typeof data.pfo_count === "number" ? data.pfo_count : "—";
    if (typeof data.last_reset === "number") {
      sysResetEl.textContent = relativeTime(data.last_reset);
      sysResetEl.title = new Date(data.last_reset * 1000).toLocaleString();
    } else {
      sysResetEl.textContent = "—";
    }
  } catch (e) {
    sysDomainEl.textContent = "unreachable";
  }
}

refreshStatus();
setInterval(refreshStatus, 20000);

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
    url: "/api/demo/upload",
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
    url: "/api/demo/upload",
    options: { method: "POST", body: new FormData(form) },
    extractClaims: (data) => data.claims,
  });
  setBusy(form, false);
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
    url: "/api/demo/insert",
    options: {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ claim_text: claimText, confidence, source }),
    },
    extractClaims: (data) => [data],
  });
  setBusy(form, false);
});
