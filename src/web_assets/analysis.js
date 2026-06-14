import { openProgressStream, startAnalysisJob } from "./api.js";
import { loadResult } from "./results.js";
import { state } from "./state.js";
import { setStatus, showStep } from "./ui.js";
import { formatMs } from "./utils.js";

export function bindAnalysisEvents() {
  document.getElementById("startAnalysis").addEventListener("click", startAnalysis);

  [
    ["minimumClusterSize", "minimum_cluster_size"],
    ["similarityThreshold", "similarity_threshold_percent"],
    ["subgroupThreshold", "subgroup_similarity_threshold_percent"],
  ].forEach(([id, key]) => {
    document.getElementById(id).addEventListener("input", (event) => {
      state.settings[key] = Number(event.target.value);
    });
  });

  [
    ["boostedTerms", "boosted"],
    ["suppressedTerms", "suppressed"],
    ["excludedTerms", "excluded"],
  ].forEach(([id, key]) => {
    document.getElementById(id).addEventListener("input", (event) => {
      state.settings.label_terms[key] = parseTermList(event.target.value);
    });
  });
}

export function syncSettings() {
  if (!state.settings.label_terms) state.settings.label_terms = { boosted: [], suppressed: [], excluded: [] };
  document.getElementById("minimumClusterSize").value = state.settings.minimum_cluster_size;
  document.getElementById("similarityThreshold").value = state.settings.similarity_threshold_percent;
  document.getElementById("subgroupThreshold").value = state.settings.subgroup_similarity_threshold_percent;
  document.getElementById("boostedTerms").value = formatTermList(state.settings.label_terms.boosted);
  document.getElementById("suppressedTerms").value = formatTermList(state.settings.label_terms.suppressed);
  document.getElementById("excludedTerms").value = formatTermList(state.settings.label_terms.excluded);
}

async function startAnalysis() {
  if (!state.source || !state.mapping) return;
  syncTermPolicyFromInputs();
  setStatus("Starting clustering analysis.");
  clearProgress();
  showStep("analysis");
  try {
    const payload = await startAnalysisJob(state.source.sourceId, state.mapping, state.settings);
    state.jobId = payload.jobId;
    listenForProgress(payload.jobId);
  } catch (error) {
    setStatus(error.message, true);
  }
}

function syncTermPolicyFromInputs() {
  if (!state.settings.label_terms) state.settings.label_terms = { boosted: [], suppressed: [], excluded: [] };
  state.settings.label_terms.boosted = parseTermList(document.getElementById("boostedTerms").value);
  state.settings.label_terms.suppressed = parseTermList(document.getElementById("suppressedTerms").value);
  state.settings.label_terms.excluded = parseTermList(document.getElementById("excludedTerms").value);
}

function parseTermList(value) {
  return [...new Set(String(value || "")
    .split(/[\n,;]+/)
    .map((term) => term.trim())
    .filter(Boolean))];
}

function formatTermList(terms) {
  return Array.isArray(terms) ? terms.join("\n") : "";
}

function listenForProgress(jobId) {
  const events = openProgressStream(jobId);
  events.onmessage = async (message) => {
    const event = JSON.parse(message.data);
    applyProgressEvent(event);
    if (event.kind === "finished") {
      events.close();
      await loadResult(jobId);
    }
    if (event.kind === "failed") {
      events.close();
      setStatus(event.message, true);
    }
  };
  events.onerror = () => setStatus("Progress stream disconnected; polling job state.", true);
}

function applyProgressEvent(event) {
  setStatus(event.message);
  if (event.progress) {
    const percent = Math.round(progressFraction(event.progress) * 100);
    const bar = document.getElementById("progressBar");
    bar.style.width = `${percent}%`;
    bar.textContent = `${percent}%`;
    document.getElementById("progressMessage").textContent = event.message;
    renderWorkers(event.progress.workers || []);
  }
  const log = document.getElementById("progressLog");
  const row = document.createElement("div");
  row.textContent = `${formatMs(event.elapsedMs)}  ${event.message}`;
  log.prepend(row);
  while (log.children.length > 80) log.lastChild.remove();
}

function clearProgress() {
  document.getElementById("progressBar").style.width = "0%";
  document.getElementById("progressBar").textContent = "0%";
  document.getElementById("progressMessage").textContent = "Waiting for progress.";
  document.getElementById("workerProgress").innerHTML = "";
  document.getElementById("progressLog").innerHTML = "";
}

function progressFraction(progress) {
  if (!progress.total_steps) return 0;
  const substep = progress.substep && progress.substep.total
    ? progress.substep.current / progress.substep.total
    : 0;
  return Math.max(0, Math.min(1, ((progress.step - 1) + substep) / progress.total_steps));
}

function renderWorkers(workers) {
  document.getElementById("workerProgress").innerHTML = workers
    .map((worker) => {
      const percent = worker.total ? Math.round((worker.completed / worker.total) * 100) : 0;
      return `<div class="worker">Worker ${worker.worker}: ${worker.completed}/${worker.total}
        <div class="mini-track"><div class="mini-bar" style="width:${percent}%"></div></div>
      </div>`;
    })
    .join("");
}
