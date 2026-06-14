export async function importSource(file) {
  const response = await fetch(`/api/import?filename=${encodeURIComponent(file.name)}`, {
    method: "POST",
    body: await file.arrayBuffer(),
  });
  return jsonOrThrow(response);
}

export async function loadWorksheet(sourceId, sheet) {
  const response = await fetch(`/api/sources/${sourceId}/worksheet?sheet=${encodeURIComponent(sheet)}`);
  return jsonOrThrow(response);
}

export async function startAnalysisJob(sourceId, mapping, settings) {
  const response = await fetch("/api/analyze", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      sourceId,
      mapping,
      settings,
    }),
  });
  return jsonOrThrow(response);
}

export function openProgressStream(jobId) {
  return new EventSource(`/api/jobs/${jobId}/events`);
}

export async function fetchResult(jobId) {
  const response = await fetch(`/api/jobs/${jobId}/result`);
  return jsonOrThrow(response);
}

export async function restoreSession(run) {
  const response = await fetch("/api/sessions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ run }),
  });
  return jsonOrThrow(response);
}

export async function fetchPivot(jobId, rowIndices, rowColumns, columnColumns) {
  const response = await fetch(`/api/jobs/${jobId}/pivot`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ rowIndices, rowColumns, columnColumns }),
  });
  return jsonOrThrow(response);
}

export function exportExcel(jobId) {
  window.location.href = `/api/jobs/${jobId}/export`;
}

export async function exportClusterViewExcel(jobId, payload) {
  await downloadPost(`/api/jobs/${jobId}/cluster-view/export`, payload, "cluster_view.xlsx");
}

export async function exportPivotExcel(jobId, rowIndices, rowColumns, columnColumns) {
  await downloadPost(`/api/jobs/${jobId}/pivot/export`, { rowIndices, rowColumns, columnColumns }, "pivot.xlsx");
}

async function downloadPost(url, payload, fallbackName) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
  if (!response.ok) throw new Error(await errorText(response));
  const blob = await response.blob();
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = downloadFileName(response, fallbackName);
  document.body.appendChild(link);
  link.click();
  link.remove();
  URL.revokeObjectURL(link.href);
}

async function errorText(response) {
  try {
    const payload = await response.json();
    return payload.error || response.statusText;
  } catch {
    return response.statusText;
  }
}

function downloadFileName(response, fallbackName) {
  const disposition = response.headers.get("Content-Disposition") || "";
  const match = disposition.match(/filename="?([^"]+)"?/i);
  return match?.[1] || fallbackName;
}

async function jsonOrThrow(response) {
  const payload = await response.json();
  if (!response.ok) throw new Error(payload.error || response.statusText);
  return payload;
}
