import {renderColumnTypes} from "./columns.js";
import { state } from "./state.js";
import { downloadJson, setStatus, showError, showStep } from "./ui.js";
import { escapeHtml } from "./utils.js";

const roles = [
  ["incident_number", "Incident number", true],
  ["short_description", "Short description", true],
  ["assignment_group", "Assignment group", false],
  ["service", "Service", false],
  ["category", "Category", false],
  ["configuration_item", "Configuration item", false],
  ["date", "Date", false],
];

export function bindMappingEvents() {
  document.getElementById("mappingInput").addEventListener("change", async (event) => {
    const file = event.target.files[0];
    if (!file) return;
    try {
      const json = JSON.parse(await file.text());
      state.mapping = json.mapping || json;
      renderMapping();
      setStatus(`Loaded mapping ${file.name}.`);
    } catch (error) {
      setStatus(error.message, true);
      showError("Mapping load failed", error.message);
    } finally {
      event.target.value = "";
    }
  });

  document.getElementById("downloadMapping").addEventListener("click", () => {
    downloadJson("incident_mapping.json", { version: 1, mapping: state.mapping });
  });

  document.getElementById("confirmMapping").addEventListener("click", () => {
    if (state.mapping?.incident_number == null || state.mapping?.short_description == null) {
      setStatus("Map both required fields before continuing.", true);
      showError("Mapping incomplete", "Map both required fields before continuing.");
      return;
    }
    setStatus("Field mapping confirmed.");
    showStep("analysis");
  });
}

// A workbook with several worksheets loads its first one; say which, and how to change it.
function renderWorksheetNotice() {
  const notice = document.getElementById("mappingNotice");
  if (!notice) return;
  notice.replaceChildren();
  const sheets = state.source?.worksheets || [];
  if (sheets.length < 2) return;
  const first = state.source.selectedWorksheet === sheets[0];
  const text = document.createElement("p");
  text.textContent = first
    ? `The source file has ${sheets.length} worksheets. The first worksheet, “${sheets[0]}”, was used by default. To use a different worksheet, go back to 1 Source and select it.`
    : `The source file has ${sheets.length} worksheets. Worksheet “${state.source.selectedWorksheet}” is used. To use a different worksheet, go back to 1 Source and select it.`;
  const back = document.createElement("button");
  back.type = "button";
  back.textContent = "Go to 1 Source";
  back.onclick = () => showStep("source");
  notice.append(text, back);
}

export function renderMapping() {
  renderWorksheetNotice();
  if(state.source)renderColumnTypes(document.getElementById("mappingColumnTypes"),state.source.headers,state.source.columnTypes);
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
