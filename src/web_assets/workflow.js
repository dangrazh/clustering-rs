import { state } from "./state.js";
import { mutateReview, fetchReview } from "./api.js";
import { escapeHtml } from "./utils.js";
import { askDialog } from "./ui.js";

export const statusLabels = {
  Review: "Review", Reviewed: "Reviewed", GetSmeFeedback: "Get SME feedback",
  PendingSmeFeedback: "Pending SME feedback", NoOpportunity: "No opportunity",
  ToBeAddressed: "To be addressed", InProgress: "In progress", Solved: "Solved",
};
export function selectionKey(selection) {
  if (selection?.type === "cluster") return String(selection.cluster);
  if (selection?.type === "theme") return `${selection.cluster}:${selection.theme}`;
  return null;
}
export function displayLabel(review, key, generated) {
  return review?.annotations?.labels?.[key] ?? generated;
}
export function effectiveWorkflow(review, key) {
  const entries = review?.annotations?.entries || {};
  return entries[key]?.workflow || entries[key?.split(":")[0]]?.workflow || { status: "Review", assignments: {}, path: ["Review"] };
}
export function stateMatches(review, key, statuses) {
  return !statuses?.length || statuses.includes(effectiveWorkflow(review, key).status);
}
let membershipCache = null;
export function workflowRows(run, review, statuses) {
  if (!statuses?.length) return null;
  const signature = [...statuses].sort().join("|");
  if (membershipCache?.run === run && membershipCache.review === review && membershipCache.signature === signature) return membershipCache.rows;
  const rows = new Set();
  for (const cluster of run.clusters || []) {
    if (stateMatches(review, String(cluster.id), statuses)) for (const row of cluster.incident_row_indices) rows.add(row);
    for (const theme of cluster.subgroups) {
      if (stateMatches(review, `${cluster.id}:${theme.id}`, statuses)) for (const row of theme.incident_row_indices) rows.add(row);
    }
  }
  membershipCache = { run, review, signature, rows };
  return rows;
}
export function applyReviewResponse(payload) {
  if(payload.analysis && payload.analysis.id!==state.analysisId)return;
  if(payload.analysis && state.shared && payload.analysis.id===state.shared.analysis.id && payload.sharedRevision<state.shared.sharedRevision)return;
  state.review = payload.review;
  state.allowedActions = payload.allowedActions || {};
  state.shared = payload.analysis ? payload : null;
  state.commentAccess=payload.commentAccess;
}
export function renderSaveStatus() {
  const target = document.getElementById("reviewSaveStatus");
  if (target && state.analysisId) {target.textContent=state.shared?.analysis.archived ? "Archived — read-only" : state.saveStatus || "Saved";return;}
  if (target) target.textContent = state.review?.annotations.revision !== state.savedReviewRevision
    ? "Unsaved review changes — use Save Session." : "Review data saved. Save a session to preserve current view settings.";
}
let workflowTarget = null;
let workflowBusy = false;
export function openWorkflow(selection, onChange) {
  workflowTarget = selectionKey(selection);
  state.presenceCluster=workflowTarget?.split(":")[0]||null;
  renderWorkflow(onChange);
  const dialog = document.getElementById("workflowDialog");
  dialog.showModal();
  if (dialog.style.left) fitWorkflowDialog(dialog);
}
function setWorkflowBounds(dialog, bounds) {
  Object.assign(dialog.style, { margin: "0", left: `${bounds.x}px`, top: `${bounds.y}px`, width: `${bounds.width}px`, height: `${bounds.height}px` });
}
function fitWorkflowDialog(dialog) {
  const bounds = dialog.getBoundingClientRect();
  const width = Math.min(bounds.width, window.innerWidth - 32);
  const height = Math.min(bounds.height, window.innerHeight - 48);
  setWorkflowBounds(dialog, { width, height, x: Math.max(16, Math.min(bounds.x, window.innerWidth - width - 16)), y: Math.max(24, Math.min(bounds.y, window.innerHeight - height - 24)) });
}
function bindWorkflowGeometry(dialog) {
  const header = dialog.querySelector(".workflow-dialog-header");
  const clamp = (value, min, max) => Math.max(min, Math.min(value, max));
  function adjust(start, dx, dy, edge) {
    const right = window.innerWidth - 16, bottom = window.innerHeight - 24;
    const minWidth = Math.min(360, right - 16), minHeight = Math.min(280, bottom - 24);
    let x = start.x, y = start.y, endX = start.right, endY = start.bottom;
    if (edge === "move") {
      x = clamp(x + dx, 16, right - start.width); y = clamp(y + dy, 24, bottom - start.height);
      endX = x + start.width; endY = y + start.height;
    } else {
      if (edge.includes("w")) x = clamp(x + dx, 16, endX - minWidth);
      if (edge.includes("e")) endX = clamp(endX + dx, x + minWidth, right);
      if (edge.includes("n")) y = clamp(y + dy, 24, endY - minHeight);
      if (edge.includes("s")) endY = clamp(endY + dy, y + minHeight, bottom);
    }
    setWorkflowBounds(dialog, { x, y, width: endX - x, height: endY - y });
  }
  function bind(handle, edge) {
    handle.addEventListener("pointerdown", event => {
      if (event.button !== 0 || event.target.closest("button")) return;
      event.preventDefault();
      const start = dialog.getBoundingClientRect(), x = event.clientX, y = event.clientY;
      handle.setPointerCapture(event.pointerId);
      dialog.classList.add("workflow-dragging");
      const move = e => adjust(start, e.clientX - x, e.clientY - y, edge);
      const stop = () => {
        dialog.classList.remove("workflow-dragging");
        handle.removeEventListener("pointermove", move);
        handle.removeEventListener("lostpointercapture", stop);
      };
      handle.addEventListener("pointermove", move);
      handle.addEventListener("lostpointercapture", stop);
    });
    handle.addEventListener("keydown", event => {
      if (event.target !== handle || !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) return;
      event.preventDefault();
      const step = event.shiftKey ? 40 : 10;
      adjust(dialog.getBoundingClientRect(), event.key === "ArrowLeft" ? -step : event.key === "ArrowRight" ? step : 0, event.key === "ArrowUp" ? -step : event.key === "ArrowDown" ? step : 0, edge);
    });
  }
  header.tabIndex = 0;
  header.setAttribute("aria-label", "Move popup using arrow keys or drag the header");
  header.title = "Drag to move; arrow keys move when focused";
  bind(header, "move");
  for (const edge of ["n", "s", "e", "w", "ne", "nw", "se", "sw"]) {
    const handle = document.createElement("span");
    handle.className = `workflow-resize workflow-resize-${edge}`;
    if (edge === "se") {
      handle.tabIndex = 0; handle.setAttribute("role", "button");
      handle.setAttribute("aria-label", "Resize popup using arrow keys or drag");
      handle.title = "Drag to resize; arrow keys resize when focused";
    } else handle.setAttribute("aria-hidden", "true");
    dialog.appendChild(handle); bind(handle, edge);
  }
  window.addEventListener("resize", () => { if (dialog.open) fitWorkflowDialog(dialog); });
}
export function bindWorkflow(onChange) {
  const dialog = document.getElementById("workflowDialog");
  bindWorkflowGeometry(dialog);
  const close = () => { if (!workflowBusy) dialog.close(); };
  document.getElementById("closeWorkflow").addEventListener("click", close);
  dialog.addEventListener("cancel", event => { if (workflowBusy) event.preventDefault(); });
  dialog.addEventListener("click", event => {
    const bounds = dialog.getBoundingClientRect();
    if (event.target === dialog && (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom)) close();
  });
  dialog.addEventListener("close", () => {
    // Native close events are queued; do not clear a popup reopened in the meantime.
    if (dialog.open) return;
    const key = workflowTarget;
    workflowTarget = null;
    state.presenceCluster=null;
    // A saved edit rebuilds the tree, so the original trigger may have been replaced.
    const trigger = [...document.querySelectorAll("[data-workflow-key]")].find(button => button.dataset.workflowKey === key);
    (trigger || document.querySelector("#clusterList .cluster-item"))?.focus();
  });
  const filters = document.getElementById("workflowFilters");
  filters.innerHTML = Object.entries(statusLabels).map(([value, label]) => `<label><input type="checkbox" value="${value}"> ${label}</label>`).join("");
  filters.addEventListener("change", () => {
    state.workflowStates = [...filters.querySelectorAll("input:checked")].map(input => input.value);
    state.detailPage = 1; onChange();
  });
  document.getElementById("clearWorkflowFilter").addEventListener("click", () => {
    state.workflowStates = []; onChange();
  });
  window.addEventListener("beforeunload", event => {
    if (state.analysisId ? state.saveStatus && state.saveStatus!=="Saved" : state.review && state.review.annotations.revision !== state.savedReviewRevision) { event.preventDefault(); event.returnValue = ""; }
  });
}
function identity() {
  if(state.user)return state.user;
  try { return JSON.parse(localStorage.getItem("incident-clustering-author-v1")) || {}; } catch { return {}; }
}
function dateText(value) { return value ? new Date(value).toLocaleString() : ""; }
const penIcon = '<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="m15 5 4 4M4 20l4-1L20 7a2.8 2.8 0 0 0-4-4L4 15z"/></svg>';
const trashIcon = '<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path d="M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7"/></svg>';
function summary(snapshot) {
  const workflow = snapshot.workflow, assignments = workflow.assignments;
  return `${statusLabels[workflow.status]}${snapshot.inherited ? " (inherited)" : ""}; SME: ${assignments.smeEmail || "—"}; Owner: ${assignments.ownerEmail || "—"}; ETA: ${assignments.eta || "—"}`;
}
export function renderWorkflow(onChange) {
  document.querySelectorAll("#workflowFilters input").forEach(input => { input.checked = state.workflowStates.includes(input.value); });
  renderSaveStatus();
  const panel = document.getElementById("workflowPanel"), key = workflowTarget;
  if (!key || !state.review?.annotations.entries[key]) return;
  const hadFocus = panel.contains(document.activeElement);
  panel.dataset.dirty="false";panel.oninput=()=>{panel.dataset.dirty="true";};
  const entry = state.review.annotations.entries[key], workflow = effectiveWorkflow(state.review, key);
  const inherited = !entry.workflow, actor = identity(), assignments = workflow.assignments;
  const allowed = state.allowedActions[key] || { next: [], canUndo: false };
  const cluster = state.analysis.clusters.find(c => String(c.id) === key.split(":")[0]);
  const generatedLabel = key.includes(":") ? cluster?.subgroups.find(t => String(t.id) === key.split(":")[1])?.label : cluster?.label;
  const label = displayLabel(state.review, key, generatedLabel);
  let base=state.shared ? structuredClone({versions:state.shared.versions,reviewers:state.shared.reviewers,commentAccess:state.shared.commentAccess}) : null;
  let conflict=null;
  panel.innerHTML = `
    <h3 tabindex="-1">${key.includes(":") ? "Theme" : "Cluster"} ${escapeHtml(key)} — <span class="workflow-label">${escapeHtml(label || key)}</span> <button id="editLabel" type="button" class="comment-icon" aria-label="Edit label" title="Edit label">${penIcon}</button> <span class="state-badge">${statusLabels[workflow.status]}</span></h3>
    <form id="labelForm" class="label-form hidden"><label>Label<input name="label" required maxlength="500"></label><div class="workflow-actions"><button>Save label</button><button id="cancelLabel" type="button">Cancel</button></div></form>
    <p>${inherited ? `Inherits workflow and assignments from cluster ${escapeHtml(key.split(":")[0])}. Editing this theme creates an independent override.` : key.includes(":") ? "Independent theme workflow." : "Changes also apply to themes that inherit this cluster."}</p>
    <details class="author-settings"><summary>Editing identity: ${escapeHtml(actor.name || "Set your name and email")}</summary>
      <form id="authorForm" class="workflow-form">
        <label>Name<input name="name" required maxlength="200" value="${escapeHtml(actor.name || "")}"></label>
        <label>Email<input name="email" type="email" required value="${escapeHtml(actor.email || "")}"></label>
        <button>Save identity</button>
      </form>
    </details>
    <form id="workflowForm" class="workflow-form">
      <label>SME email<input name="smeEmail" type="email" value="${escapeHtml(assignments.smeEmail || "")}"></label>
      <label>Implementation owner<input name="ownerEmail" type="email" value="${escapeHtml(assignments.ownerEmail || "")}"></label>
      <label>ETA<input name="eta" type="date" value="${escapeHtml(assignments.eta || "")}"></label>
      <label>Next state<select name="next"><option value="">Keep current state</option>${allowed.next.map(s => `<option value="${s}">${statusLabels[s]}</option>`).join("")}</select></label>
      <div class="workflow-actions"><button>Save workflow</button>
      <button type="button" id="undoWorkflow" ${allowed.canUndo ? "" : "disabled"}>Undo last transition</button>
      ${key.includes(":") && !inherited ? '<button type="button" id="inheritWorkflow">Return to cluster inheritance</button>' : ""}</div>
    </form>
    <p id="workflowError" role="alert" class="workflow-error"></p>
    <details open><summary>Comments (${entry.comments.length})</summary>
      <div id="commentList">${entry.comments.map(c => `<article class="review-comment" data-comment="${escapeHtml(c.id)}"><p class="comment-text">${escapeHtml(c.text)}</p><div class="comment-meta"><small>${escapeHtml(c.author.name)} &lt;${escapeHtml(c.author.email)}&gt; · ${escapeHtml(dateText(c.createdAt))}${c.updatedAt ? ` · edited ${escapeHtml(dateText(c.updatedAt))}` : ""}</small><div class="comment-actions"><button type="button" class="comment-icon" data-edit="${escapeHtml(c.id)}" aria-label="Edit comment" title="Edit comment">${penIcon}</button><button type="button" class="comment-icon" data-delete="${escapeHtml(c.id)}" aria-label="Delete comment" title="Delete comment">${trashIcon}</button></div></div></article>`).join("")}</div>
      <form id="commentForm"><label>New comment<textarea name="text" required maxlength="100000" rows="3"></textarea></label><button>Add comment</button></form>
    </details>
    <details><summary>Workflow history</summary><div id="workflowHistory"></div></details>`;
  const error = message => { document.getElementById("workflowError").textContent = message; };
  document.getElementById("authorForm").addEventListener("submit", event => {
    event.preventDefault(); const values = new FormData(event.target);
    try { localStorage.setItem("incident-clustering-author-v1", JSON.stringify({ name: values.get("name").trim(), email: values.get("email").trim() })); renderWorkflow(onChange); }
    catch { error("Cannot store editing identity in this browser. Enable local storage and retry."); }
  });
  let busy = false;
  async function mutate(action) {
    if (busy) return;
    if (conflict) {
      busy = true;
      const accepted = await askDialog("Review conflicting change", `This item changed. Current values:\n${JSON.stringify(conflict.review.annotations.entries[key],null,2)}\nCurrent label: ${conflict.review.annotations.labels[key] || "generated label"}\n\nSubmit your retained draft using these current values?`, {confirmLabel:"Submit draft"});
      busy = false;
      if (!accepted) return;
      base=structuredClone({versions:conflict.versions,reviewers:conflict.reviewers,commentAccess:conflict.commentAccess});conflict=null;
    }
    const actor = identity();
    if (!actor.name || !actor.email) { panel.querySelector(".author-settings").open = true; error("Set your editing name and email first."); return; }
    busy = true;
    workflowBusy = true;
    document.getElementById("closeWorkflow").disabled = true;
    const controls = [...panel.querySelectorAll("button")]; controls.forEach(button => { button.disabled = true; });
    try {
      applyReviewResponse(await mutateReview(state.jobId, { revision: state.review.annotations.revision, target: action.type==="reviewer"?key.split(":")[0]:key, actor, action,base }));
      onChange();
    } catch (e) {
      if (e.snapshot) {conflict=e.snapshot;applyReviewResponse(e.snapshot);}
      // Preserve entered text on failure. Refresh metadata without overwriting the form.
      try { applyReviewResponse(await fetchReview(state.jobId)); } catch { /* Keep original error. */ }
      error(e.message); controls.forEach(button => { button.disabled = false; });
    } finally { busy = false; workflowBusy = false; document.getElementById("closeWorkflow").disabled = false; }
  }
  const labelForm = document.getElementById("labelForm");
  document.getElementById("editLabel").addEventListener("click", () => {
    labelForm.classList.remove("hidden"); labelForm.elements.label.value = label || "";
    labelForm.elements.label.focus(); labelForm.elements.label.select();
  });
  const cancelLabel = () => { labelForm.classList.add("hidden"); document.getElementById("editLabel").focus(); };
  document.getElementById("cancelLabel").addEventListener("click", cancelLabel);
  labelForm.addEventListener("keydown", event => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); cancelLabel(); } });
  labelForm.addEventListener("submit", event => { event.preventDefault(); mutate({ type: "rename", label: labelForm.elements.label.value }); });
  const form = document.getElementById("workflowForm");
  const requiredFields = () => {
    const status = form.elements.next.value || workflow.status;
    form.elements.smeEmail.required = ["PendingSmeFeedback", "ToBeAddressed", "InProgress", "Solved"].includes(status);
    form.elements.ownerEmail.required = form.elements.eta.required = ["InProgress", "Solved"].includes(status);
  };
  form.elements.next.addEventListener("change", requiredFields); requiredFields();
  form.addEventListener("submit", event => {
    event.preventDefault(); const values = new FormData(form);
    const assignments = { smeEmail: values.get("smeEmail").trim() || null, ownerEmail: values.get("ownerEmail").trim() || null, eta: values.get("eta") || null };
    const status = values.get("next"); mutate(status ? { type: "transition", status, assignments } : { type: "assign", assignments });
  });
  document.getElementById("undoWorkflow").addEventListener("click", () => mutate({ type: "undo" }));
  document.getElementById("inheritWorkflow")?.addEventListener("click", () => {
    if (confirm("Replace this theme's independent workflow and assignments with its cluster's current values? Comments and history will remain.")) mutate({ type: "inherit" });
  });
  document.getElementById("commentForm").addEventListener("submit", event => { event.preventDefault(); mutate({ type: "addComment", text: new FormData(event.target).get("text") }); });
  panel.querySelectorAll("[data-delete]").forEach(button => button.addEventListener("click", () => {
    if (confirm("Delete this comment? Deleted comments are not retained in history.")) mutate({ type: "deleteComment", id: button.dataset.delete });
  }));
  panel.querySelectorAll("[data-edit]").forEach(button => button.addEventListener("click", () => {
    const comment = entry.comments.find(c => c.id === button.dataset.edit), article = button.closest("article");
    article.innerHTML = '<form class="edit-comment"><label>Edit comment<textarea name="text" required maxlength="100000" rows="4"></textarea></label><button>Save comment</button> <button type="button">Cancel</button></form>';
    article.querySelector("textarea").value = comment.text;
    article.querySelector("form").addEventListener("submit", event => { event.preventDefault(); mutate({ type: "editComment", id: comment.id, text: new FormData(event.target).get("text") }); });
    article.querySelector('button[type="button"]').addEventListener("click", () => renderWorkflow(onChange));
  }));
  if(state.user){
    const author=document.createElement("p");author.textContent=`Editing as ${state.user.name}`;panel.querySelector(".author-settings").replaceWith(author);
  }
  if(state.commentAccess)panel.querySelectorAll("[data-edit],[data-delete]").forEach(button=>{if(!state.commentAccess[button.dataset.edit||button.dataset.delete]?.canEdit)button.remove();});
  if(state.shared){
    const parent=key.split(":")[0],reviewer=state.shared.reviewers[parent]?.value;
    const container=document.createElement("div");container.className="workflow-actions reviewer-assignment";
    const label=document.createElement("label");label.textContent="Cluster reviewer ";const select=document.createElement("select");label.append(select);
    select.add(new Option("Unassigned",""));for(const user of state.users||[])select.add(new Option(user.name,user.id));select.value=reviewer?.userId||"";
    const assign=document.createElement("button");assign.type="button";assign.textContent="Assign";assign.onclick=()=>mutate({type:"reviewer",userId:select.value||null});
    const claim=document.createElement("button");claim.type="button";claim.textContent="Claim";claim.onclick=()=>mutate({type:"reviewer",userId:state.user.id});
    container.append(label,assign,claim);
    if(reviewer?.unconfirmed){const note=document.createElement("span");note.textContent=`Imported reviewer: ${reviewer.recorded?.name||"unknown"} (unconfirmed)`;container.append(note);}
    panel.querySelector("h3").after(container);
    panel.querySelectorAll("[data-edit],[data-delete]").forEach(button=>{if(!state.shared.commentAccess[button.dataset.edit||button.dataset.delete]?.canEdit)button.remove();});
    if(state.shared.analysis.archived)panel.querySelectorAll("button").forEach(button=>button.disabled=true);
  }
  const events = entry.history.map(event => ({ event, source: key }));
  if (inherited) for (const event of state.review.annotations.entries[key.split(":")[0]].history) events.push({ event, source: key.split(":")[0] });
  events.sort((a,b) => b.event.timestamp.localeCompare(a.event.timestamp));
  document.getElementById("workflowHistory").innerHTML = events.length ? events.map(({event, source}) => `<article class="history-event"><strong>${escapeHtml(event.action)} · ${escapeHtml(dateText(event.timestamp))}</strong><p>${escapeHtml(event.actor.name)} &lt;${escapeHtml(event.actor.email)}&gt;${source !== key ? ` · inherited from cluster ${escapeHtml(source)}` : ""}</p><p>Before: ${escapeHtml(summary(event.before))}</p><p>After: ${escapeHtml(summary(event.after))}</p></article>`).join("") : '<p class="muted">No workflow changes yet.</p>';
  if (hadFocus) panel.querySelector("h3").focus();
  for(const event of (state.shared?.audit||[]).filter(e=>e.entity===key||e.entity===key.split(":")[0]).sort((a,b)=>b.timestamp.localeCompare(a.timestamp))){
    const article=document.createElement("article");article.className="history-event";
    article.textContent=`${event.action} · ${dateText(event.timestamp)} · ${event.actor.name}: ${JSON.stringify(event.before)} → ${JSON.stringify(event.after)}`;
    document.getElementById("workflowHistory").append(article);
  }
}

export function refreshWorkflow(onChange){
  const panel=document.getElementById("workflowPanel");
  if(!workflowBusy&&panel.dataset.dirty!=="true")renderWorkflow(onChange);
  if(state.shared?.analysis.archived){
    panel.querySelectorAll("button").forEach(button=>{if(!button.hasAttribute("data-before-archive"))button.dataset.beforeArchive=String(button.disabled);button.disabled=true;});
  }else{
    panel.querySelectorAll("[data-before-archive]").forEach(button=>{button.disabled=button.dataset.beforeArchive==="true";delete button.dataset.beforeArchive;});
  }
}
