import { escapeHtml } from "./utils.js";

// Native prompts cannot share the application's typography or color variables.
export function askDialog(title, message, { value, label = "Analysis name", confirmLabel = "Continue", cancel = true } = {}) {
  return new Promise(resolve => {
    const dialog = document.createElement("dialog"); dialog.className = "app-dialog compact";
    const heading = document.createElement("h2"); heading.id = `dialog-${crypto.randomUUID()}`; heading.textContent = title;
    dialog.setAttribute("aria-labelledby", heading.id);
    const text = document.createElement("p"); text.className = "dialog-message"; text.textContent = message;
    const form = document.createElement("form");
    let input;
    if (value !== undefined) {
      const field = document.createElement("label"); field.className = "field"; field.textContent = label;
      input = document.createElement("input"); input.value = value; input.required = true; input.maxLength = 200;
      field.append(input); form.append(field);
    }
    const actions = document.createElement("div"); actions.className = "dialog-actions";
    if (cancel) { const button = document.createElement("button"); button.type = "button"; button.textContent = "Cancel"; button.onclick = () => dialog.close(); actions.append(button); }
    const submit = document.createElement("button"); submit.type = "submit"; submit.className = "primary"; submit.textContent = confirmLabel; actions.append(submit);
    form.append(actions); dialog.append(heading, text, form);
    let result = null;
    form.onsubmit = event => { event.preventDefault(); if(input && !input.value.trim()){input.focus(); return;} result = input ? input.value.trim() : true; dialog.close(); };
    dialog.addEventListener("close", () => { dialog.remove(); resolve(result); }, {once:true});
    document.body.append(dialog); dialog.showModal(); (input || submit).focus(); input?.select();
  });
}
export const noticeDialog = (message) => askDialog("Analysis update", message, {cancel:false, confirmLabel:"Close"});

export function bindNavigation() {
  document.querySelectorAll(".steps button").forEach((button) => {
    button.addEventListener("click", () => showStep(button.dataset.step));
  });
  document.getElementById("statusOverlayClose")?.addEventListener("click", hideOverlay);
}

export function showStep(step) {
  window.dispatchEvent(new CustomEvent("pane-change", {detail:step}));
  document.querySelectorAll(".screen").forEach((screen) => {
    screen.classList.toggle("active", screen.id === step);
  });
  document.querySelectorAll(".steps button").forEach((button) => {
    button.classList.toggle("active", button.dataset.step === step);
  });
}

export function setStatus(message, isError = false) {
  const status = document.getElementById("statusLine");
  status.textContent = message;
  status.style.color = isError ? "var(--danger)" : "var(--muted)";
}

export function showBusy(title, message) {
  showOverlay(title, message, false);
}

export function showError(title, message) {
  showOverlay(title, message, true);
}

export function hideOverlay() {
  document.getElementById("statusOverlay")?.classList.add("hidden");
}

function showOverlay(title, message, isError) {
  const overlay = document.getElementById("statusOverlay");
  if (!overlay) return;
  overlay.classList.toggle("error", isError);
  document.getElementById("statusOverlayTitle").textContent = title;
  document.getElementById("statusOverlayMessage").textContent = message;
  document.getElementById("statusOverlayClose").classList.toggle("hidden", !isError);
  overlay.classList.remove("hidden");
}

export function statsHtml(items) {
  return items.map(([label, value]) => `<span class="stat">${label}: <strong>${escapeHtml(value)}</strong></span>`).join("");
}

export function downloadJson(fileName, value) {
  const blob = new Blob([JSON.stringify(value, null, 2)], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = fileName;
  link.click();
  URL.revokeObjectURL(link.href);
}

export function renderTable(targetId, headers, rows) {
  const target = document.getElementById(targetId);
  if (!headers.length) {
    target.innerHTML = "";
    return;
  }
  target.innerHTML = `<table><thead><tr>${headers
    .map((header) => `<th>${escapeHtml(header)}</th>`)
    .join("")}</tr></thead><tbody>${rows
    .map(
      (row) =>
        `<tr>${headers
          .map((_, column) => `<td title="${escapeHtml(row[column] || "")}">${escapeHtml(row[column] || "")}</td>`)
          .join("")}</tr>`
    )
    .join("")}</tbody></table>`;
}
