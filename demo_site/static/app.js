const tabButtons = document.querySelectorAll(".tab-btn");
const tabPanels = document.querySelectorAll(".tab-panel");
tabButtons.forEach((btn) => {
  btn.addEventListener("click", () => {
    tabButtons.forEach((b) => b.classList.remove("active"));
    tabPanels.forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById(`tab-${btn.dataset.tab}`).classList.add("active");
  });
});

const resultsEl = document.getElementById("results");

function setBusy(form, busy) {
  form.querySelector("button").disabled = busy;
}

function renderError(message) {
  resultsEl.innerHTML = `<p class="error-msg">${escapeHtml(message)}</p>`;
}

function renderClaims(claims) {
  if (!claims || claims.length === 0) {
    resultsEl.innerHTML = `<p class="hint">No claims were extracted.</p>`;
    return;
  }
  resultsEl.innerHTML = claims.map(renderCard).join("");
}

function renderCard(c) {
  const pct = typeof c.confidence === "number" ? Math.round(c.confidence * 100) : null;
  const badges = [];
  if (c.status && c.status !== "inserted") {
    badges.push(`<span class="badge ${c.status}">${escapeHtml(c.status)}</span>`);
  }
  if (c.conflict_count) {
    badges.push(`<span class="badge conflict">&#9888; conflict detected (${c.conflict_count})</span>`);
  }
  if (c.corroboration_count) {
    badges.push(`<span class="badge corroborated">corroborated &times;${c.corroboration_count}</span>`);
  }
  return `
    <div class="card">
      <div class="claim">${escapeHtml(c.claim_text || "")}</div>
      <div class="meta">
        ${pct !== null ? `<span>confidence: ${pct}%</span>` : ""}
        ${c.source ? `<span>source: ${escapeHtml(c.source)}</span>` : ""}
        ${c.reason ? `<span>${escapeHtml(c.reason)}</span>` : ""}
        ${badges.join(" ")}
      </div>
    </div>`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (m) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  }[m]));
}

async function submitForm(form, buildRequest, extractClaims) {
  setBusy(form, true);
  resultsEl.innerHTML = `<p class="hint">Working&hellip; (embedding + conflict detection takes a few seconds)</p>`;
  try {
    const { url, options } = buildRequest(form);
    const resp = await fetch(url, options);
    const data = await resp.json();
    if (!resp.ok) {
      renderError(data.error || "something went wrong");
      return;
    }
    renderClaims(extractClaims(data));
  } catch (e) {
    renderError("network error — try again");
  } finally {
    setBusy(form, false);
  }
}

document.getElementById("pdf-form").addEventListener("submit", (e) => {
  e.preventDefault();
  submitForm(
    e.target,
    (form) => ({ url: "/api/demo/upload", options: { method: "POST", body: new FormData(form) } }),
    (data) => data.claims
  );
});

document.getElementById("text-form").addEventListener("submit", (e) => {
  e.preventDefault();
  submitForm(
    e.target,
    (form) => ({ url: "/api/demo/upload", options: { method: "POST", body: new FormData(form) } }),
    (data) => data.claims
  );
});

document.getElementById("claim-form").addEventListener("submit", (e) => {
  e.preventDefault();
  submitForm(
    e.target,
    (form) => {
      const fd = new FormData(form);
      return {
        url: "/api/demo/insert",
        options: {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            claim_text: fd.get("claim_text"),
            confidence: parseFloat(fd.get("confidence")),
            source: fd.get("source"),
          }),
        },
      };
    },
    (data) => [data]
  );
});
