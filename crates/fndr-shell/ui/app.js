const stateElement = document.querySelector("#state");
const detailElement = document.querySelector("#detail");
const observedAtElement = document.querySelector("#observed-at");
const tickElement = document.querySelector("#tick");
const drainElement = document.querySelector("#drain");
const startCaptureElement = document.querySelector("#start-capture");
const pauseToggleElement = document.querySelector("#pause-toggle");
const openAuditLogElement = document.querySelector("#open-audit-log");
const auditDetailElement = document.querySelector("#audit-detail");
const auditEntriesElement = document.querySelector("#audit-entries");
const screenRecordingAccessElement = document.querySelector("#screen-recording-access");
const searchFormElement = document.querySelector("#search-form");
const searchQueryElement = document.querySelector("#search-query");
const runSearchElement = document.querySelector("#run-search");
const searchDetailElement = document.querySelector("#search-detail");
const searchRouteElement = document.querySelector("#search-route");
const searchResultsElement = document.querySelector("#search-results");

/* What each VectorRouteState means for the person reading the results. A
 * keyword-only answer is never presented as the whole answer: every state
 * below renders, including the healthy one. */
const VECTOR_ROUTE_NOTES = {
  available: {
    state: "running",
    label: "Keyword + semantic",
    detail: "Both local retrieval routes ran for this query.",
  },
  model_missing: {
    state: "blocked",
    label: "Keyword only",
    detail:
      "No local embedding model is installed, so semantic matches were not searched. These are exact-text matches only.",
  },
  index_missing: {
    state: "blocked",
    label: "Keyword only",
    detail:
      "The semantic index does not exist yet; it is built the first time capture flushes what it has stored. These are exact-text matches only.",
  },
  failed: {
    state: "failed",
    label: "Keyword only",
    detail:
      "The semantic route failed on this query and FNDR logged the reason locally. These are exact-text matches only.",
  },
};

function words(value) {
  return String(value || "unknown").replaceAll("_", " ");
}

function formatTimestamp(value) {
  return Number.isFinite(value) && value > 0
    ? new Date(value).toLocaleString()
    : "Not reported yet";
}

function statusDetail(status, state) {
  if (status.tick?.reason === "screen_recording_or_capture_unavailable") {
    return "Screen Recording or capture tooling is unavailable. Check FNDR's Screen Recording permission in System Settings, then wait for the next capture opportunity.";
  }
  if (status.tick?.reason === "private_browsing") {
    return "FNDR withheld this capture because the foreground metadata signaled private browsing. Leave the private window before expecting a new capture.";
  }
  if (status.reason === "model_missing") {
    return "The configured local embedding model is missing. Choose a valid --model path, then reopen FNDR.";
  }
  return status.reason
    ? `Host reported: ${words(status.reason)}.`
    : state === "running"
      ? "The local host is running. Latest tick details stay content-free."
      : "The local host has not reported a blocking reason.";
}

function renderStatus(status) {
  const state = String(status.state || "unknown").toLowerCase();
  stateElement.textContent = words(state);
  stateElement.dataset.state = state;
  detailElement.textContent = statusDetail(status, state);
  observedAtElement.textContent = formatTimestamp(status.observed_at_ms);
  tickElement.textContent = status.tick
    ? `${words(status.tick.capture)}; ${words(status.tick.flush)}${
        status.tick.reason ? ` (${words(status.tick.reason)})` : ""
      }`
    : "No capture result reported";
  drainElement.textContent = Number.isFinite(status.shutdown_flushed_chunks)
    ? `${status.shutdown_flushed_chunks} chunk(s) flushed`
    : "Not reported";
  const canTogglePause = state === "running" || state === "paused";
  startCaptureElement.disabled = !(state === "stopped" && status.reason === "not_started");
  pauseToggleElement.disabled = !canTogglePause;
  pauseToggleElement.textContent = state === "paused" ? "Resume capture" : "Pause capture";
}

function renderAuditEntries(entries) {
  auditEntriesElement.replaceChildren();
  auditEntriesElement.hidden = entries.length === 0;
  if (entries.length === 0) {
    auditDetailElement.textContent = "No local MCP tool calls have been recorded yet.";
    return;
  }

  auditDetailElement.textContent = `${entries.length} most recent local MCP call(s).`;
  for (const entry of entries) {
    const item = document.createElement("li");
    const released = entry.raw_released ? "raw text released" : "no raw text released";
    item.textContent = `${formatTimestamp(entry.at_ms)} — ${entry.tool} (${entry.outcome}; ${released})`;
    auditEntriesElement.append(item);
  }
}

/* SQLite's FTS5 `snippet()` wraps each matched term in `[` `]` (see
 * fndr-store's keyword query); the vector route's snippets carry no markers
 * at all. Rendered with `textContent` only, never `innerHTML`, so captured
 * text can never inject markup. */
function renderSnippet(container, text) {
  container.replaceChildren();
  const parts = String(text || "").split(/(\[[^\]]*\])/g);
  for (const part of parts) {
    if (part.startsWith("[") && part.endsWith("]") && part.length >= 2) {
      const mark = document.createElement("mark");
      mark.textContent = part.slice(1, -1);
      container.append(mark);
    } else if (part) {
      container.append(document.createTextNode(part));
    }
  }
}

function renderRouteNote(vectorRoute) {
  const note = VECTOR_ROUTE_NOTES[vectorRoute] || VECTOR_ROUTE_NOTES.model_missing;
  searchRouteElement.hidden = false;
  searchRouteElement.replaceChildren();
  const pill = document.createElement("span");
  pill.className = "pill";
  pill.dataset.state = note.state;
  pill.textContent = note.label;
  searchRouteElement.append(pill, document.createTextNode(note.detail));
}

function renderSearchResults(results) {
  renderRouteNote(results.vector_route);
  searchResultsElement.replaceChildren();

  if (results.vault === "not_created") {
    searchDetailElement.textContent = "FNDR has not captured anything on this machine yet.";
    searchResultsElement.hidden = true;
    return;
  }

  if (results.hits.length === 0) {
    searchDetailElement.textContent = `No local matches for "${results.query}".`;
    searchResultsElement.hidden = true;
    return;
  }

  searchDetailElement.textContent = `${results.hits.length} local match(es) for "${results.query}".`;
  searchResultsElement.hidden = false;
  for (const hit of results.hits) {
    const item = document.createElement("li");
    item.className = "list-item";

    const snippet = document.createElement("p");
    snippet.className = "result-snippet";
    renderSnippet(snippet, hit.snippet);

    const meta = document.createElement("p");
    meta.className = "result-meta";
    const routeLabel = hit.route === "vector" ? "Semantic match" : "Keyword match";
    meta.textContent = `${routeLabel} • ${hit.app_name || "Unknown app"} • ${formatTimestamp(hit.captured_at_ms)}`;

    item.append(snippet, meta);
    searchResultsElement.append(item);
  }
}

function renderScreenRecordingAccess(access) {
  if (access === "granted") {
    screenRecordingAccessElement.textContent = "Granted. This check did not request capture.";
    return;
  }
  if (access === "not_granted") {
    screenRecordingAccessElement.textContent = "Not granted. Selecting Start capture is the only action that can begin the macOS permission flow.";
    return;
  }
  screenRecordingAccessElement.textContent = "Unavailable. FNDR could not read the local preflight state.";
}

async function attachCaptureStatus() {
  const tauri = window.__TAURI__;
  if (!tauri?.core?.invoke || !tauri?.event?.listen) {
    stateElement.textContent = "Unavailable";
    stateElement.dataset.state = "failed";
    detailElement.textContent = "The local FNDR bridge is unavailable in this window.";
    return;
  }

  // Search is read-only and independent of capture: it is enabled as soon as
  // the bridge exists, regardless of whether capture status below succeeds.
  searchQueryElement.disabled = false;
  runSearchElement.disabled = false;

  try {
    renderStatus(await tauri.core.invoke("capture_status"));
    openAuditLogElement.disabled = false;
    await tauri.event.listen("capture://status", (event) => renderStatus(event.payload));
  } catch (error) {
    stateElement.textContent = "Unavailable";
    stateElement.dataset.state = "failed";
    detailElement.textContent = "FNDR could not read its local capture status.";
    console.error("FNDR capture status bridge failed", error);
  }

  try {
    renderScreenRecordingAccess(await tauri.core.invoke("screen_recording_preflight"));
  } catch (error) {
    renderScreenRecordingAccess("unavailable");
    console.error("FNDR Screen Recording preflight failed", error);
  }
}

openAuditLogElement.addEventListener("click", async () => {
  const tauri = window.__TAURI__;
  openAuditLogElement.disabled = true;
  try {
    renderAuditEntries(await tauri.core.invoke("recent_audit_entries"));
  } catch (error) {
    auditDetailElement.textContent = "FNDR could not read the local audit ledger.";
    auditEntriesElement.hidden = true;
    console.error("FNDR audit log bridge failed", error);
  } finally {
    openAuditLogElement.disabled = false;
  }
});

startCaptureElement.addEventListener("click", async () => {
  const tauri = window.__TAURI__;
  startCaptureElement.disabled = true;
  try {
    renderStatus(await tauri.core.invoke("start_capture"));
  } catch (error) {
    detailElement.textContent = "FNDR could not start its local capture host.";
    console.error("FNDR start capture failed", error);
  }
});

pauseToggleElement.addEventListener("click", async () => {
  const tauri = window.__TAURI__;
  const shouldPause = stateElement.dataset.state !== "paused";
  pauseToggleElement.disabled = true;
  try {
    renderStatus(
      await tauri.core.invoke("set_capture_paused", { paused: shouldPause }),
    );
  } catch (error) {
    detailElement.textContent = "FNDR could not change its local capture state.";
    console.error("FNDR pause control failed", error);
  }
});

searchFormElement.addEventListener("submit", async (event) => {
  event.preventDefault();
  const tauri = window.__TAURI__;
  const query = searchQueryElement.value.trim();
  if (!query) {
    return;
  }

  runSearchElement.disabled = true;
  searchDetailElement.textContent = "Searching your local memory…";
  searchRouteElement.hidden = true;
  searchResultsElement.hidden = true;
  try {
    renderSearchResults(await tauri.core.invoke("search_memories", { query }));
  } catch (error) {
    searchDetailElement.textContent = "FNDR could not search the local vault.";
    searchRouteElement.hidden = true;
    searchResultsElement.hidden = true;
    console.error("FNDR search failed", error);
  } finally {
    runSearchElement.disabled = false;
  }
});

void attachCaptureStatus();
