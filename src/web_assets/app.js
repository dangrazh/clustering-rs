const state = {
  source: null,
  mapping: null,
  settings: {
    minimum_cluster_size: 50,
    similarity_threshold_percent: 42,
    subgroup_similarity_threshold_percent: 58,
  },
  jobId: null,
  analysis: null,
  selection: null,
  expandedClusters: new Set(),
  detailPage: 1,
  detailPageSize: 50,
  resultsTreeWidth: null,
  detailColumnWidths: [],
  detailColumnOrder: [],
};

const roles = [
  ["incident_number", "Incident number", true],
  ["short_description", "Short description", true],
  ["assignment_group", "Assignment group", false],
  ["service", "Service", false],
  ["category", "Category", false],
  ["configuration_item", "Configuration item", false],
  ["date", "Date", false],
];

document.addEventListener("DOMContentLoaded", () => {
  bindNavigation();
  bindInputs();
  bindResultsSplitter();
  syncSettings();
});

function bindNavigation() {
  document.querySelectorAll(".steps button").forEach((button) => {
    button.addEventListener("click", () => showStep(button.dataset.step));
  });
}

function bindInputs() {
  document.getElementById("fileInput").addEventListener("change", async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    setStatus(`Uploading ${file.name}...`);
    try {
      const response = await fetch(`/api/import?filename=${encodeURIComponent(file.name)}`, {
        method: "POST",
        body: await file.arrayBuffer(),
      });
      acceptSource(await jsonOrThrow(response));
      showStep("mapping");
    } catch (error) {
      setStatus(error.message, true);
    }
  });

  document.getElementById("mappingInput").addEventListener("change", async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    const json = JSON.parse(await file.text());
    state.mapping = json.mapping || json;
    renderMapping();
    setStatus(`Loaded mapping ${file.name}.`);
  });

  document.getElementById("downloadMapping").addEventListener("click", () => {
    downloadJson("incident_mapping.json", { version: 1, mapping: state.mapping });
  });

  document.getElementById("confirmMapping").addEventListener("click", () => {
    if (state.mapping?.incident_number == null || state.mapping?.short_description == null) {
      setStatus("Map both required fields before continuing.", true);
      return;
    }
    setStatus("Field mapping confirmed.");
    showStep("analysis");
  });

  document.getElementById("downloadSession").addEventListener("click", () => {
    if (!state.analysis) return;
    downloadJson("incident_analysis_session.json", { version: 1, run: state.analysis });
  });

  document.getElementById("exportExcel").addEventListener("click", () => {
    if (state.jobId) window.location.href = `/api/jobs/${state.jobId}/export`;
  });

  document.getElementById("startAnalysis").addEventListener("click", startAnalysis);

  document.getElementById("pageSize").addEventListener("change", (event) => {
    state.detailPageSize = Number(event.target.value);
    state.detailPage = 1;
    renderDetailRows();
  });

  document.getElementById("previousPage").addEventListener("click", () => {
    if (state.detailPage > 1) {
      state.detailPage -= 1;
      renderDetailRows();
    }
  });

  document.getElementById("nextPage").addEventListener("click", () => {
    const totalPages = Math.max(1, Math.ceil(detailRowIndices().length / state.detailPageSize));
    if (state.detailPage < totalPages) {
      state.detailPage += 1;
      renderDetailRows();
    }
  });

  window.addEventListener("resize", () => applyResultsPaneWidth());

  [
    ["minimumClusterSize", "minimum_cluster_size"],
    ["similarityThreshold", "similarity_threshold_percent"],
    ["subgroupThreshold", "subgroup_similarity_threshold_percent"],
  ].forEach(([id, key]) => {
    document.getElementById(id).addEventListener("input", (event) => {
      state.settings[key] = Number(event.target.value);
    });
  });
}

function acceptSource(source) {
  state.source = source;
  state.mapping = source.suggestedMapping;
  state.analysis = null;
  state.jobId = null;
  state.selection = null;
  document.querySelector('[data-step="mapping"]').disabled = false;
  document.querySelector('[data-step="analysis"]').disabled = false;
  document.querySelector('[data-step="results"]').disabled = true;
  document.getElementById("sourceStatus").textContent = `${source.rowCount} rows, ${source.headers.length} columns`;
  setStatus(`Loaded ${source.rowCount} rows from ${source.fileName}.`);
  renderSource();
  renderMapping();
}

function renderSource() {
  const source = state.source;
  document.getElementById("sourceStats").innerHTML = statsHtml([
    ["Rows", source.rowCount],
    ["Columns", source.headers.length],
    ["File", source.fileName],
  ]);

  const worksheetBar = document.getElementById("worksheetBar");
  worksheetBar.innerHTML = "";
  worksheetBar.classList.toggle("hidden", source.worksheets.length === 0);
  source.worksheets.forEach((sheet) => {
    const button = document.createElement("button");
    button.textContent = sheet;
    if (sheet === source.selectedWorksheet) button.classList.add("primary");
    button.addEventListener("click", () => selectWorksheet(sheet));
    worksheetBar.appendChild(button);
  });

  renderTable("previewTable", source.headers, source.previewRows);
}

async function selectWorksheet(sheet) {
  setStatus(`Loading worksheet ${sheet}...`);
  try {
    const response = await fetch(
      `/api/sources/${state.source.sourceId}/worksheet?sheet=${encodeURIComponent(sheet)}`
    );
    acceptSource(await jsonOrThrow(response));
  } catch (error) {
    setStatus(error.message, true);
  }
}

function renderMapping() {
  const source = state.source;
  if (!source || !state.mapping) return;
  const form = document.getElementById("mappingForm");
  form.innerHTML = "";
  roles.forEach(([key, label, required]) => {
    const wrapper = document.createElement("label");
    wrapper.className = "field";
    wrapper.textContent = `${label}${required ? " *" : ""}`;
    const select = document.createElement("select");
    select.innerHTML = `<option value="">Not mapped</option>${source.headers
      .map((header, index) => `<option value="${index}">${escapeHtml(header)}</option>`)
      .join("")}`;
    select.value = state.mapping[key] ?? "";
    select.addEventListener("change", () => {
      state.mapping[key] = select.value === "" ? null : Number(select.value);
    });
    wrapper.appendChild(select);
    form.appendChild(wrapper);
  });

  const additional = document.getElementById("additionalText");
  additional.innerHTML = "";
  source.headers.forEach((header, index) => {
    const label = document.createElement("label");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = state.mapping.additional_text.includes(index);
    checkbox.addEventListener("change", () => {
      const values = new Set(state.mapping.additional_text);
      checkbox.checked ? values.add(index) : values.delete(index);
      state.mapping.additional_text = [...values].sort((a, b) => a - b);
    });
    label.append(checkbox, header);
    additional.appendChild(label);
  });
}

function syncSettings() {
  document.getElementById("minimumClusterSize").value = state.settings.minimum_cluster_size;
  document.getElementById("similarityThreshold").value = state.settings.similarity_threshold_percent;
  document.getElementById("subgroupThreshold").value = state.settings.subgroup_similarity_threshold_percent;
}

async function startAnalysis() {
  if (!state.source || !state.mapping) return;
  setStatus("Starting clustering analysis.");
  clearProgress();
  showStep("analysis");
  try {
    const response = await fetch("/api/analyze", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        sourceId: state.source.sourceId,
        mapping: state.mapping,
        settings: state.settings,
      }),
    });
    const payload = await jsonOrThrow(response);
    state.jobId = payload.jobId;
    listenForProgress(payload.jobId);
  } catch (error) {
    setStatus(error.message, true);
  }
}

function listenForProgress(jobId) {
  const events = new EventSource(`/api/jobs/${jobId}/events`);
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

async function loadResult(jobId) {
  const response = await fetch(`/api/jobs/${jobId}/result`);
  state.analysis = await jsonOrThrow(response);
  state.selection = { type: "all" };
  state.expandedClusters = new Set();
  state.detailPage = 1;
  state.detailColumnWidths = [];
  state.detailColumnOrder = [];
  document.querySelector('[data-step="results"]').disabled = false;
  renderResults();
  showStep("results");
}

function renderResults() {
  const run = state.analysis;
  applyResultsPaneWidth();
  document.getElementById("resultStats").innerHTML = statsHtml([
    ["Clusters", run.clusters.length],
    ["Processed", run.processed_incidents.length],
    ["Ignored", run.ignored_rows.length],
    ["Unclustered", run.unclustered_row_indices.length],
  ]);
  renderClusterList();
  renderDetailRows();
}

function renderClusterList() {
  const list = document.getElementById("clusterList");
  list.innerHTML = "";
  addClusterButton(list, "All incidents", { type: "all" });
  state.analysis.clusters.forEach((cluster) => {
    const key = clusterKey(cluster.id);
    const expanded = state.expandedClusters.has(key);
    const row = document.createElement("div");
    row.className = "tree-row";

    const toggle = document.createElement("button");
    toggle.className = "tree-toggle";
    toggle.type = "button";
    toggle.textContent = expanded ? "v" : ">";
    toggle.setAttribute("aria-label", expanded ? "Collapse cluster" : "Expand cluster");
    toggle.setAttribute("aria-expanded", String(expanded));
    toggle.disabled = cluster.subgroups.length === 0;
    toggle.addEventListener("click", () => {
      if (expanded) {
        state.expandedClusters.delete(key);
      } else {
        state.expandedClusters.add(key);
      }
      renderClusterList();
    });

    row.appendChild(toggle);
    addClusterButton(
      row,
      `${clusterId(cluster.id)} - ${cluster.label} (${cluster.incident_row_indices.length})`,
      { type: "cluster", cluster: cluster.id },
      "cluster-label"
    );
    list.appendChild(row);

    if (expanded) {
      cluster.subgroups.forEach((theme) => {
        addClusterButton(
          list,
          `Theme ${theme.id} - ${theme.label} (${theme.incident_row_indices.length})`,
          { type: "theme", cluster: cluster.id, theme: theme.id },
          "theme"
        );
      });
    }
  });
}

function addClusterButton(list, text, selection, extraClass = "") {
  const button = document.createElement("button");
  button.className = `cluster-item ${extraClass}`;
  if (sameSelection(selection, state.selection)) button.classList.add("active");
  button.textContent = text;
  button.addEventListener("click", () => {
    state.selection = selection;
    state.detailPage = 1;
    renderClusterList();
    renderDetailRows();
  });
  list.appendChild(button);
}

function renderDetailRows() {
  const run = state.analysis;
  const rowIndices = detailRowIndices();
  const totalRows = rowIndices.length;
  const pageSize = state.detailPageSize;
  const totalPages = Math.max(1, Math.ceil(totalRows / pageSize));
  state.detailPage = Math.min(Math.max(1, state.detailPage), totalPages);
  const start = (state.detailPage - 1) * pageSize;
  const pageRows = rowIndices.slice(start, start + pageSize);
  const rows = pageRows.map((index) => run.source.rows[index]).filter(Boolean);
  renderTable("detailTable", run.source.headers, rows);
  renderPagination(totalRows, start, rows.length, totalPages);
}

function detailRowIndices() {
  const run = state.analysis;
  if (!run) return [];
  let rowIndices = run.processed_incidents.map((record) => record.source_row_index);
  if (state.selection?.type === "cluster") {
    const cluster = run.clusters.find((item) => item.id === state.selection.cluster);
    rowIndices = cluster?.incident_row_indices || [];
  }
  if (state.selection?.type === "theme") {
    const cluster = run.clusters.find((item) => item.id === state.selection.cluster);
    const theme = cluster?.subgroups.find((item) => item.id === state.selection.theme);
    rowIndices = theme?.incident_row_indices || [];
  }
  return rowIndices;
}

function renderPagination(totalRows, start, shownRows, totalPages) {
  const first = totalRows === 0 ? 0 : start + 1;
  const last = start + shownRows;
  document.getElementById("pageStatus").textContent =
    `${first}-${last} of ${totalRows} records, page ${state.detailPage} of ${totalPages}`;
  document.getElementById("previousPage").disabled = state.detailPage <= 1;
  document.getElementById("nextPage").disabled = state.detailPage >= totalPages;
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

function renderTable(targetId, headers, rows) {
  const target = document.getElementById(targetId);
  if (!headers.length) {
    target.innerHTML = "";
    return;
  }
  const resizable = targetId === "detailTable";
  const columnOrder = resizable ? detailColumnOrder(headers.length) : headers.map((_, index) => index);
  const colgroup = resizable
    ? `<colgroup>${columnOrder
        .map((column) => {
          const width = state.detailColumnWidths[column];
          return `<col${width ? ` style="width:${width}px"` : ""}>`;
        })
        .join("")}</colgroup>`
    : "";
  target.innerHTML = `<table>${colgroup}<thead><tr>${columnOrder
    .map((column, position) => {
      const dragAttrs = resizable ? ` draggable="true" data-column="${column}" data-position="${position}"` : "";
      const handle = resizable
        ? `<span class="column-resizer" data-column="${column}" data-position="${position}"></span>`
        : "";
      return `<th${dragAttrs}>${escapeHtml(headers[column])}${handle}</th>`;
    })
    .join("")}</tr></thead><tbody>${rows
    .map(
      (row) =>
        `<tr>${columnOrder
          .map((column) => `<td title="${escapeHtml(row[column] || "")}">${escapeHtml(row[column] || "")}</td>`)
          .join("")}</tr>`
    )
    .join("")}</tbody></table>`;
  if (resizable) {
    bindColumnResizers(target);
    bindColumnDrag(target);
  }
}

function detailColumnOrder(columnCount) {
  if (state.detailColumnOrder.length !== columnCount) {
    state.detailColumnOrder = Array.from({ length: columnCount }, (_, index) => index);
  }
  return state.detailColumnOrder;
}

function bindResultsSplitter() {
  const splitter = document.getElementById("resultsSplitter");
  const layout = document.querySelector(".results-layout");
  if (!splitter || !layout) return;

  splitter.addEventListener("mousedown", (event) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = document.getElementById("clusterList").getBoundingClientRect().width;
    document.body.classList.add("is-resizing");

    const onMove = (moveEvent) => {
      const bounds = layout.getBoundingClientRect();
      const minWidth = 280;
      const maxWidth = Math.max(minWidth, bounds.width - 420);
      state.resultsTreeWidth = Math.min(maxWidth, Math.max(minWidth, startWidth + moveEvent.clientX - startX));
      applyResultsPaneWidth();
    };

    const onUp = () => {
      document.body.classList.remove("is-resizing");
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  });
}

function applyResultsPaneWidth() {
  const layout = document.querySelector(".results-layout");
  if (!layout || !state.resultsTreeWidth) return;
  const bounds = layout.getBoundingClientRect();
  const minWidth = 280;
  const maxWidth = Math.max(minWidth, bounds.width - 420);
  state.resultsTreeWidth = Math.min(maxWidth, Math.max(minWidth, state.resultsTreeWidth));
  layout.style.gridTemplateColumns = `${state.resultsTreeWidth}px 8px minmax(0, 1fr)`;
}

function bindColumnResizers(target) {
  target.querySelectorAll(".column-resizer").forEach((handle) => {
    handle.addEventListener("mousedown", (event) => {
      event.preventDefault();
      event.stopPropagation();
      const column = Number(handle.dataset.column);
      const position = Number(handle.dataset.position);
      const table = target.querySelector("table");
      const col = table?.querySelectorAll("col")[position];
      const th = handle.closest("th");
      const startX = event.clientX;
      const startWidth = state.detailColumnWidths[column] || th.getBoundingClientRect().width;
      document.body.classList.add("is-resizing");

      const onMove = (moveEvent) => {
        const width = Math.max(80, startWidth + moveEvent.clientX - startX);
        state.detailColumnWidths[column] = width;
        if (col) col.style.width = `${width}px`;
      };

      const onUp = () => {
        document.body.classList.remove("is-resizing");
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };

      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    });
  });
}

function bindColumnDrag(target) {
  target.querySelectorAll("th[draggable='true']").forEach((header) => {
    header.addEventListener("dragstart", (event) => {
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", header.dataset.column);
      header.classList.add("dragging");
    });

    header.addEventListener("dragend", () => {
      target.querySelectorAll("th").forEach((item) => item.classList.remove("dragging", "drag-over"));
    });

    header.addEventListener("dragover", (event) => {
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      header.classList.add("drag-over");
    });

    header.addEventListener("dragleave", () => {
      header.classList.remove("drag-over");
    });

    header.addEventListener("drop", (event) => {
      event.preventDefault();
      const sourceColumn = Number(event.dataTransfer.getData("text/plain"));
      const targetColumn = Number(header.dataset.column);
      moveDetailColumn(sourceColumn, targetColumn);
      renderDetailRows();
    });
  });
}

function moveDetailColumn(sourceColumn, targetColumn) {
  if (sourceColumn === targetColumn) return;
  const order = [...state.detailColumnOrder];
  const sourceIndex = order.indexOf(sourceColumn);
  const targetIndex = order.indexOf(targetColumn);
  if (sourceIndex < 0 || targetIndex < 0) return;
  const [moved] = order.splice(sourceIndex, 1);
  order.splice(targetIndex, 0, moved);
  state.detailColumnOrder = order;
}

function showStep(step) {
  document.querySelectorAll(".screen").forEach((screen) => {
    screen.classList.toggle("active", screen.id === step);
  });
  document.querySelectorAll(".steps button").forEach((button) => {
    button.classList.toggle("active", button.dataset.step === step);
  });
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

async function jsonOrThrow(response) {
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.error || response.statusText);
  return payload;
}

function statsHtml(items) {
  return items.map(([label, value]) => `<span class="stat">${label}: <strong>${escapeHtml(value)}</strong></span>`).join("");
}

function setStatus(message, isError = false) {
  const status = document.getElementById("statusLine");
  status.textContent = message;
  status.style.color = isError ? "var(--danger)" : "var(--muted)";
}

function downloadJson(fileName, value) {
  const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = fileName;
  link.click();
  URL.revokeObjectURL(link.href);
}

function clusterId(id) {
  if (id && typeof id === "object" && "0" in id) id = id[0];
  if (Array.isArray(id)) id = id[0];
  return id === 0 ? "Unclustered" : `C${String(id).padStart(4, "0")}`;
}

function clusterKey(id) {
  if (id && typeof id === "object" && "0" in id) id = id[0];
  if (Array.isArray(id)) id = id[0];
  return String(id);
}

function sameSelection(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function formatMs(ms) {
  if (ms >= 60000) return `${Math.floor(ms / 60000)}m ${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}s`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
