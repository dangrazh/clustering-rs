import { importSource, loadWorksheet } from "./api.js";
import { renderMapping } from "./mapping.js";
import { loadSavedSession } from "./results.js";
import { state } from "./state.js";
import { renderTable, setStatus, showStep, statsHtml } from "./ui.js";

export function bindSourceEvents() {
  const fileInput = document.getElementById("fileInput");
  const sessionInput = document.getElementById("sessionInput");
  if (!fileInput || !sessionInput) {
    setStatus("Source controls did not initialize. Refresh the page to reload the latest UI.", true);
    return;
  }
  console.info("Source file controls initialized.");

  fileInput.addEventListener("change", async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    setStatus(`Uploading ${file.name}...`);
    try {
      acceptSource(await importSource(file));
      showStep("mapping");
    } catch (error) {
      setStatus(error.message, true);
    } finally {
      event.target.value = "";
    }
  });

  const loadSession = async (event) => {
    if (sessionInput.dataset.loading === "true") return;
    console.info("Session file selection event received.", {
      type: event.type,
      value: event.target.value,
      files: event.target.files ? Array.from(event.target.files).map((file) => `${file.name}|${file.size}`) : [],
    });
    const file = event.target.files && event.target.files[0];
    if (!file) {
      setStatus("Session file was selected, but the browser did not expose it to the app. Try selecting the file again or use a standard file picker.", true);
      return;
    }
    sessionInput.dataset.loading = "true";
    setStatus(`Loading session ${file.name}...`);
    try {
      const payload = JSON.parse(await readTextFile(file));
      await loadSavedSession(payload);
      setStatus(`Loaded session ${file.name}.`);
    } catch (error) {
      setStatus(error.message, true);
    } finally {
      event.target.value = "";
      delete sessionInput.dataset.loading;
    }
  };
  sessionInput.addEventListener("change", loadSession);
  sessionInput.addEventListener("input", loadSession);
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

function readTextFile(file) {
  if (typeof file.text === "function") return file.text();
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result || ""));
    reader.onerror = () => reject(reader.error || new Error("Failed to read file."));
    reader.readAsText(file);
  });
}
