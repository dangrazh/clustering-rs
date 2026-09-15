import { state } from "./state.js";
let renewal = null;
async function renewSession(userId) {
  if (!renewal) renewal = new Promise(resolve => {
    const dialog = document.createElement("dialog");
    dialog.id = "sessionRenewal";
    dialog.className = "app-dialog compact";
    const title = document.createElement("h2"); title.id = "sessionRenewalTitle"; title.textContent = "Sign in again";
    dialog.setAttribute("aria-labelledby", title.id);
    const message = document.createElement("p");
    message.textContent = "Your session expired. Your pending edits are still here. Sign in again in a new tab to continue saving.";
    const link = document.createElement("a");
    link.href = "/auth/login"; link.target = "_blank"; link.rel = "noopener";
    link.textContent = "Sign in again";
    link.className = "button primary";
    const actions = document.createElement("div"); actions.className = "dialog-actions"; actions.append(link);
    dialog.append(title, message, actions); document.body.append(dialog); dialog.showModal();
    dialog.addEventListener("cancel", event => event.preventDefault());
    let busy = false;
    const timer = setInterval(async () => {
      if (busy) return; busy = true;
      try {
        const response = await globalThis.fetch("/api/me", { cache: "no-store" });
        if (!response.ok) return;
        const session = await response.json();
        if (session.user.id !== userId) {
          message.textContent = "A different account is signed in. Pending edits will not be submitted as that person. Sign in again with the original account to continue.";
          return;
        }
        state.csrf = session.csrf;
        clearInterval(timer); dialog.close(); dialog.remove(); resolve();
      } catch { /* Keep drafts and retry when the service returns. */ }
      finally { busy = false; }
    }, 1000);
  }).finally(() => { renewal = null; });
  return renewal;
}
export async function request(url, options = {}) {
  const userId = state.user?.id;
  while (true) {
    const headers = new Headers(options.headers);
    if (state.csrf && (options.method || "GET") !== "GET") headers.set("X-CSRF-Token", state.csrf);
    const response = await globalThis.fetch(url, { ...options, headers });
    if (response.status !== 401 || !userId || url === "/auth/logout") return response;
    state.saveStatus = "Not saved";
    window.dispatchEvent(new Event("shared-save-state"));
    await renewSession(userId);
    // Reuse the exact payload, expected versions and command ID after renewal.
  }
}
async function fetch(url,options) {
  if (state.analysisId && url.startsWith(`/api/jobs/${state.jobId}/`)) url=url.replace(`/api/jobs/${state.jobId}/`,`/api/analyses/${state.analysisId}/`);
  return request(url,options);
}
export async function importSource(file) {
  const response = await fetch(`/api/import?filename=${encodeURIComponent(file.name)}`, {
    method: "POST",
    body: await file.arrayBuffer(),
  });
  return jsonOrThrow(response);
}

export async function loadWorksheet(sourceId, sheet) {
  const response = await fetch(`/api/sources/${sourceId}/worksheet?sheet=${encodeURIComponent(sheet)}`,{method:"POST"});
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

export async function restoreSession(file) {
  const response = await fetch("/api/sessions", {
    method: "POST",
    headers: { "Content-Type": "application/octet-stream" },
    body: file,
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

export async function exportExcel(jobId, rowIndices) {
  await downloadPost(`/api/jobs/${jobId}/incidents/export`, { rowIndices }, "clustered_incidents.xlsx");
}

export async function fetchReview(jobId) {
  return jsonOrThrow(await fetch(`/api/jobs/${jobId}/review`));
}
export async function mutateReview(jobId, payload) {
  if(state.analysisId){
    const {action,target}=payload, base=payload.base;
    const expected=action.type==="rename"?base.versions[target]?.label:action.type==="reviewer"?base.reviewers[target.split(":")[0]]?.version:action.type.endsWith("Comment")?base.commentAccess[action.id]?.version:base.versions[target]?.workflow;
    const command={commandId:crypto.randomUUID(),target,action,expected:expected||0,parentVersion:base.versions[target.split(":")[0]]?.workflow||0};
    const url=`/api/analyses/${state.analysisId}/commands`;
    state.saveStatus="Saving";window.dispatchEvent(new Event("shared-save-state"));
    while(true){
      try {
        const response=await request(url,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(command)});
        if(response.status>=500) throw new TypeError("The service is unavailable");
        const data=await response.json();
        if(!response.ok){const error=new Error(data.error);error.snapshot=data.snapshot;error.status=response.status;throw error;}
        state.saveStatus="Saved";window.dispatchEvent(new Event("shared-save-state"));return data.snapshot;
      }catch(error){
        state.saveStatus="Not saved";window.dispatchEvent(new Event("shared-save-state"));
        if(!(error instanceof TypeError)) throw error;
        await new Promise(resolve=>setTimeout(resolve,2000));
      }
    }
  }
  return jsonOrThrow(await fetch(`/api/jobs/${jobId}/review/mutate`, {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(payload),
  }));
}
export async function saveSession(jobId, view) {
  return downloadPost(`/api/jobs/${jobId}/session/save`, view, "incident_analysis.icas");
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
