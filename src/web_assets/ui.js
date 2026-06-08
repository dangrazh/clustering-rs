import { escapeHtml } from "./utils.js";

export function bindNavigation() {
  document.querySelectorAll(".steps button").forEach((button) => {
    button.addEventListener("click", () => showStep(button.dataset.step));
  });
}

export function showStep(step) {
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
