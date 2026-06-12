import { importSource, loadWorksheet } from "./api.js";
import { renderMapping } from "./mapping.js";
import { loadSavedSession } from "./results.js";
import { state } from "./state.js";
import { renderTable, setStatus, showStep, statsHtml } from "./ui.js";

export function bindSourceEvents() {
  document.getElementById("fileInput").addEventListener("change", async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    setStatus(`Uploading ${file.name}...`);
    try {
      acceptSource(await importSource(file));
      showStep("mapping");
    } catch (error) {
      setStatus(error.message, true);
    }
  });

  document.getElementById("sessionInput").addEventListener("change", async (event) => {
    const file = event.target.files?.[0];
    if (!file) return;
    setStatus(`Loading session ${file.name}...`);
    try {
      const payload = JSON.parse(await file.text());
      await loadSavedSession(payload);
      setStatus(`Loaded session ${file.name}.`);
    } catch (error) {
      setStatus(error.message, true);
    } finally {
      event.target.value = "";
    }
  });
}

export function acceptSource(source) {
  state.source = source;
  state.mapping = source.suggestedMapping;
  state.analysis = null;
  state.jobId = null;
  state.selection = null;
  document.querySelector('[data-step="mapping"]').disabled = false;
  document.querySelector('[data-step="analysis"]').disabled = false;
  document.querySelector('[data-step="results"]').disabled = true;
  document.querySelector('[data-step="pivot"]').disabled = true;
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
    acceptSource(await loadWorksheet(state.source.sourceId, sheet));
  } catch (error) {
    setStatus(error.message, true);
  }
}
