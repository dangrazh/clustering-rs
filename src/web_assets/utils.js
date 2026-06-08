export function clusterId(id) {
  if (id && typeof id === "object" && "0" in id) id = id[0];
  if (Array.isArray(id)) id = id[0];
  return id === 0 ? "Unclustered" : `C${String(id).padStart(4, "0")}`;
}

export function clusterKey(id) {
  if (id && typeof id === "object" && "0" in id) id = id[0];
  if (Array.isArray(id)) id = id[0];
  return String(id);
}

export function sameSelection(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function formatMs(ms) {
  if (ms >= 60000) return `${Math.floor(ms / 60000)}m ${String(Math.floor((ms % 60000) / 1000)).padStart(2, "0")}s`;
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${ms}ms`;
}

export function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
