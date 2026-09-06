// sivtr web UI: session browser + search over the unified archive.
// Plain ES module; the JSON API is documented in docs-site (web UI section).

const $ = (id) => document.getElementById(id);

const PAGE = 200;
const inflight = { sessions: 0, search: 0, detail: 0 };
const controllers = { sessions: null, search: null, detail: null };

function abort(key) {
  controllers[key]?.abort();
  controllers[key] = new AbortController();
  inflight[key] += 1;
  return { signal: controllers[key].signal, token: inflight[key] };
}

function stillCurrent(key, token) {
  return inflight[key] === token;
}

async function api(path, signal) {
  const response = await fetch(path, signal ? { signal } : undefined);
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || `${response.status} ${response.statusText}`);
  }
  return response.json();
}

function el(tag, attrs = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value);
  }
  for (const child of children) node.append(child);
  return node;
}

function fmtTime(value) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function shortText(value, max = 90) {
  const text = (value || "").replace(/\s+/g, " ").trim();
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

function activateOnEnter(node, action) {
  node.addEventListener("keydown", (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      action();
    }
  });
}

async function loadProviders() {
  const list = $("sessions");
  try {
    const providers = await api("/api/v1/providers");
    const select = $("provider");
    for (const item of providers) {
      select.append(
        el("option", { value: item.provider, text: `${item.provider} (${item.sessions})` })
      );
    }
  } catch (error) {
    list.replaceChildren(el("li", { class: "empty", text: `Failed to load providers: ${error.message}` }));
  }
}

async function loadSessions({ append = false } = {}) {
  const { signal, token } = abort("sessions");
  const provider = $("provider").value;
  const list = $("sessions");
  const offset = append ? list.querySelectorAll("li.item").length : 0;
  const params = new URLSearchParams({ limit: String(PAGE), offset: String(offset) });
  if (provider) params.set("provider", provider);
  if (!append) {
    list.replaceChildren(el("li", { class: "empty", text: "Loading…" }));
  }
  let sessions;
  try {
    sessions = await api(`/api/v1/sessions?${params}`, signal);
  } catch (error) {
    if (error.name === "AbortError" || !stillCurrent("sessions", token)) return;
    list.replaceChildren(el("li", { class: "empty", text: `Failed to load sessions: ${error.message}` }));
    return;
  }
  if (!stillCurrent("sessions", token)) return;
  if (!append) list.replaceChildren();
  list.querySelector("li.load-more")?.remove();
  if (!append && !sessions.length) {
    list.append(el("li", { class: "empty", text: "No archived sessions yet. Run a command or use an agent, then wait for the next sync." }));
    return;
  }
  for (const session of sessions) {
    const open = () => openSession(session.provider, session.session_id);
    const item = el("li", {
      class: "item",
      role: "button",
      tabindex: "0",
      onclick: open,
    }, [
      el("span", { class: "title", text: session.title || session.session_id }),
      el("span", {
        class: "meta",
        text: `${session.provider} · ${session.record_count} records · ${fmtTime(session.ended_at || session.started_at)}`,
      }),
    ]);
    activateOnEnter(item, open);
    list.append(item);
  }
  if (sessions.length === PAGE) {
    const more = el("li", { class: "item load-more", role: "button", tabindex: "0", text: "Load more" });
    const load = () => loadSessions({ append: true });
    more.addEventListener("click", load);
    activateOnEnter(more, load);
    list.append(more);
  }
}

async function loadUsage() {
  const total = $("usage-total");
  const groups = $("usage-groups");
  try {
    const payload = await api("/api/v1/usage");
    const summary = payload.summary;
    total.textContent = `${summary.totals.cost} · ${summary.totals.requests} requests · ${summary.totals.unpriced_requests} unpriced`;
    groups.replaceChildren();
    for (const group of summary.groups.slice(-14).reverse()) {
      groups.append(el("div", { class: "usage-row" }, [
        el("span", { text: `${group.day} · ${group.provider} · ${group.model || "unknown"}` }),
        el("span", { class: "meta", text: `${group.requests} · ${group.cost}` }),
      ]));
    }
    if (payload.warnings.length) {
      groups.append(el("div", { class: "warning", text: `${payload.warnings.length} source refresh warning(s)` }));
    }
  } catch (error) {
    total.textContent = `Usage unavailable: ${error.message}`;
  }
}

async function loadStats() {
  const target = $("stats-summary");
  try {
    const payload = await api("/api/v1/stats");
    const stats = payload.stats;
    target.textContent = `${stats.sessions} sessions · ${stats.records} records · ${stats.active_days} active days · ${stats.starred_sessions} starred`;
  } catch (error) {
    target.textContent = `Stats unavailable: ${error.message}`;
  }
}

async function openSession(provider, sessionId) {
  const { signal, token } = abort("detail");
  const detail = $("detail");
  detail.replaceChildren(el("p", { class: "empty", text: "Loading…" }));
  let payload;
  try {
    payload = await api(`/api/v1/sessions/${encodeURIComponent(provider)}/${encodeURIComponent(sessionId)}`, signal);
  } catch (error) {
    if (error.name === "AbortError" || !stillCurrent("detail", token)) return;
    detail.replaceChildren(el("p", { class: "empty", text: `Failed to load: ${error.message}` }));
    return;
  }
  if (!stillCurrent("detail", token)) return;
  const { session, records } = payload;
  detail.replaceChildren();
  detail.append(
    el("div", { class: "session-head" }, [
      el("h2", { text: session.title || session.session_id }),
      el("span", {
        class: "meta",
        text: `${session.provider} · ${session.record_count} records · ${fmtTime(session.started_at)} → ${fmtTime(session.ended_at)}`,
      }),
    ])
  );
  for (const record of records) {
    detail.append(renderRecord(record));
  }
}

function renderRecord(record) {
  const copy = () => navigator.clipboard?.writeText(record.work_ref);
  const ref = el("span", {
    class: "ref",
    role: "button",
    tabindex: "0",
    text: record.work_ref,
    title: "Click to copy ref",
    onclick: copy,
  });
  activateOnEnter(ref, copy);
  const head = el("div", { class: "record-head" }, [
    ref,
    el("span", { text: fmtTime(record.time && (record.time.ended_at || record.time.started_at)) }),
  ]);
  const body = el("div");
  for (const part of record.parts || []) {
    body.append(
      el("div", { class: `part kind-${part.kind}` }, [
        el("span", { class: "kind", text: part.label || part.kind }),
        el("pre", { text: partText(part) }),
      ])
    );
  }
  return el("article", { class: "record" }, [head, body]);
}

function partText(part) {
  if (part.kind === "action") {
    const blocks = (part.output || []).map((block) => contentText(block.content));
    if (blocks.length) return blocks.join("\n\n");
    return contentText(part.input);
  }
  return contentText(part.content);
}

function contentText(content) {
  if (!content) return "";
  // `WorkContent` serializes externally tagged: {"Text": {...}} | {"Json": ...}.
  if (content.Json !== undefined) return JSON.stringify(content.Json, null, 2);
  return content.Text?.content || "";
}

// "codex/abc/1" → "codex"; "terminal/s/1" → "terminal".
function recordNamespace(record) {
  return String(record.work_ref || "").split("/")[0] || "terminal";
}

async function runSearch() {
  const q = $("search").value.trim();
  const source = $("source").value;
  const method = $("method").value;
  $("results").classList.toggle("hidden", !q && source === "all");
  $("results-head").classList.toggle("hidden", !q && source === "all");
  if (!q) return;
  const { signal, token } = abort("search");
  const params = new URLSearchParams({ q, source, limit: "50" });
  if (method === "semantic") params.set("semantic", "true");
  if (method === "hybrid") params.set("hybrid", "true");
  const list = $("results");
  list.replaceChildren(el("li", { class: "empty", text: "Searching…" }));
  let payload;
  try {
    payload = await api(`/api/v1/search?${params}`, signal);
  } catch (error) {
    if (error.name === "AbortError" || !stillCurrent("search", token)) return;
    list.replaceChildren(el("li", { class: "empty", text: `Search failed: ${error.message}` }));
    return;
  }
  if (!stillCurrent("search", token)) return;
  list.replaceChildren();
  if (!payload.records.length) {
    list.append(el("li", { class: "empty", text: "No hits." }));
    return;
  }
  for (const record of payload.records) {
    const open = () => openSession(recordNamespace(record), record.session.canonical_id || record.session.id);
    const item = el("li", {
      class: "item",
      role: "button",
      tabindex: "0",
      onclick: open,
    }, [
      el("span", { class: "title", text: shortText(record.title) }),
      el("span", { class: "meta", text: `${record.work_ref} · ${fmtTime(record.time && (record.time.ended_at || record.time.started_at))}` }),
    ]);
    activateOnEnter(item, open);
    list.append(item);
  }
}

function showSessionsPane() {
  $("results").classList.add("hidden");
  $("results-head").classList.add("hidden");
}

$("search").addEventListener("keydown", (event) => {
  if (event.key === "Enter") runSearch();
});
$("source").addEventListener("change", () => {
  if ($("search").value.trim()) runSearch();
});
$("method").addEventListener("change", () => {
  if ($("search").value.trim()) runSearch();
});
$("provider").addEventListener("change", () => {
  showSessionsPane();
  loadSessions();
});
$("clear-search").addEventListener("click", () => {
  $("search").value = "";
  showSessionsPane();
});
document.addEventListener("keydown", (event) => {
  if (event.key === "/" && document.activeElement !== $("search")) {
    event.preventDefault();
    $("search").focus();
  }
});

loadProviders()
  .then(() => loadSessions())
  .catch((error) => {
    $("sessions").replaceChildren(el("li", { class: "empty", text: `Failed to start: ${error.message}` }));
  });
loadUsage();
loadStats();
