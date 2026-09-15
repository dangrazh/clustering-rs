import { state } from "./state.js";
import { request,fetchReview } from "./api.js";
import { loadResult,renderResults } from "./results.js";
import { applyReviewResponse,renderSaveStatus,refreshWorkflow } from "./workflow.js";
import { captureView } from "./view-state.js";
import { listenForProgress } from "./analysis.js";
import { askDialog, noticeDialog } from "./ui.js";

async function json(url,options){const response=await request(url,options);const data=await response.json();if(!response.ok)throw new Error(data.error||response.statusText);return data;}
const post=value=>({method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(value)});
let previousView="",viewBusy=false,saveCommand=null;
const viewId=crypto.randomUUID();
export async function initializeCollaboration(){
  const bar=document.createElement("section");bar.className="collaboration-bar";
  bar.innerHTML='<span id="signedInUser"></span> <button id="openLibrary">Analyses</button> <button id="centralSave">Save centrally</button> <span id="sharedTitle"></span> <span id="personalSaveStatus"></span> <button id="signOut">Sign out</button>';
  document.querySelector("header").after(bar);
  const dialog=document.createElement("dialog");dialog.id="analysisLibrary";dialog.className="app-dialog";dialog.setAttribute("aria-labelledby","libraryTitle");
  dialog.innerHTML='<h2 id="libraryTitle">Shared analyses</h2><form id="analysisSearch"><label class="field">Name <input name="search"></label> <label class="archive-filter"><input name="archived" type="checkbox"> Show archive</label> <button>Search</button></form><p id="libraryError" class="workflow-error" role="alert"></p><div id="analysisList"></div><div class="dialog-actions"><button id="closeLibrary">Close</button></div>';
  document.body.append(dialog);
  const error=document.getElementById("libraryError");
  const report=e=>{error.textContent=e.message;};
  async function refresh(){
    const form=document.getElementById("analysisSearch");
    const entries=await json(`/api/analyses?search=${encodeURIComponent(form.elements.search.value)}&archived=${form.elements.archived.checked}`);
    const list=document.getElementById("analysisList");list.replaceChildren();
    for(const analysis of entries){
      const row=document.createElement("p"),open=document.createElement("button");open.textContent=analysis.name;row.append(open);
      open.onclick=async()=>{try{await openAnalysis(analysis.id);dialog.close();}catch(e){report(e);}};
      for(const action of analysis.archived?["restore"]:["renameAnalysis","archive"]){
        const button=document.createElement("button");button.textContent={restore:"Restore",renameAnalysis:"Rename",archive:"Archive"}[action];row.append(" ",button);
        button.onclick=async()=>{
          try{
            let name;
            if(action==="renameAnalysis"){name=await askDialog("Rename analysis","Choose a unique analysis name.",{value:analysis.name,confirmLabel:"Rename"});if(!name)return;}
            if(action==="archive"&&!await askDialog("Archive analysis","Everyone currently reviewing this analysis will be notified and editing will stop. Unsaved edits must wait until restoration.",{confirmLabel:"Archive"}))return;
            await json(`/api/analyses/${analysis.id}/commands`,post({commandId:crypto.randomUUID(),target:"",expected:analysis.version,action:{type:action,name}}));await refresh();
          }catch(e){report(e);}
        };
      }
      list.append(row);
    }
  }
  document.getElementById("openLibrary").onclick=()=>{dialog.showModal();refresh().catch(report);};
  document.getElementById("closeLibrary").onclick=()=>dialog.close();
  document.getElementById("analysisSearch").onsubmit=e=>{e.preventDefault();refresh().catch(report);};
  document.getElementById("centralSave").onclick=async()=>{
    if(!state.jobId||state.analysisId)return;
    const name=await askDialog("Save analysis centrally","Choose a unique name for this shared analysis.",{value:saveCommand?.name||"",confirmLabel:"Save"});if(!name)return;
    if(!saveCommand||saveCommand.name!==name||saveCommand.jobId!==state.jobId)saveCommand={name,jobId:state.jobId,commandId:crypto.randomUUID(),view:captureView(state)};
    const button=document.getElementById("centralSave");button.disabled=true;
    document.querySelector("main").inert=true;
    try{const result=await json(`/api/jobs/${state.jobId}/save-central`,post(saveCommand));await openAnalysis(result.id);saveCommand=null;}
    catch(e){noticeDialog(`Analysis was not confirmed saved: ${e.message}. Your temporary result remains available; retry with the same name.`);}
    finally{button.disabled=!!state.analysisId;document.querySelector("main").inert=false;}
  };
  document.getElementById("signOut").onclick=async()=>{await request("/auth/logout",post({}));location.reload();};
  try{
    const session=await json("/api/me");state.user=session.user;state.csrf=session.csrf;
    document.getElementById("signedInUser").textContent=session.user.name;
    state.users=await json("/api/users");
  }catch{
    bar.replaceChildren();const link=document.createElement("a");link.href="/auth/login";link.textContent="Sign in";link.className="button primary";bar.append(link);
    document.querySelector("main").inert=true;return;
  }
  const events=new EventSource("/api/changes");let refreshing=false,refreshAgain=false;
  async function refreshShared(){
    if(refreshing){refreshAgain=true;return;}refreshing=true;
    try{
      do{refreshAgain=false;const aid=state.analysisId;if(!aid||state.loadingAnalysis)break;const snapshot=await fetchReview(state.jobId);
        if(aid!==state.analysisId)continue;
        const wasArchived=state.shared?.analysis.archived;applyReviewResponse(snapshot);renderResults(true);refreshWorkflow(()=>renderResults());
        if(snapshot.analysis.archived&&!wasArchived)noticeDialog("This analysis has been archived. Pending edits remain in the form for copying and cannot be saved until restoration.");
        renderSaveStatus();
      }while(refreshAgain);
    }finally{refreshing=false;}
  }
  events.onmessage=event=>{const data=JSON.parse(event.data);if(data.analysisId===state.analysisId)refreshShared().catch(()=>{});if(dialog.open&&["catalog","renameAnalysis","archive","restore"].includes(data.change.kind))refresh().catch(report);};
  events.onopen=()=>refreshShared().catch(()=>{});
  window.addEventListener("shared-save-state",renderSaveStatus);
  const presence=document.createElement("span");presence.id="analysisPresence";bar.append(presence);
  const jobsButton=document.createElement("button");jobsButton.textContent="My jobs";bar.append(jobsButton);
  const jobsDialog=document.createElement("dialog");jobsDialog.id="jobsDialog";jobsDialog.className="app-dialog";jobsDialog.setAttribute("aria-labelledby","jobsTitle");document.body.append(jobsDialog);
  async function showJobs(){
    jobsDialog.replaceChildren();const title=document.createElement("h2");title.id="jobsTitle";title.textContent="My jobs";jobsDialog.append(title);
    for(const job of await json("/api/jobs")){
      const row=document.createElement("p");row.className="job-row";const description=document.createElement("span");description.className="job-description";row.append(description);description.textContent=`${new Date(job.created).toLocaleString()} — ${job.state}${job.error?`: ${job.error}`:""} `;
      if(job.expires)description.append(`Available until ${new Date(job.expires*1000).toLocaleString()}. `);
      const summary=job.metadata?.summary||{},details=document.createElement("span");details.className="job-metadata";
      const count=value=>Number.isInteger(value)?value.toLocaleString():"Not recorded";
      const rows=Number.isInteger(summary.processedRows)?`Rows processed: ${count(summary.processedRows)} of ${count(summary.sourceRows)}`:`Source rows: ${count(summary.sourceRows)}`;
      details.textContent=`Source: ${summary.sourceFileName||"Not recorded"} · ${rows} · Columns: ${count(summary.columnCount)}`;
      description.append(details);
      const open=document.createElement("button");open.textContent=job.state==="finished"?"Open result":"View progress";row.append(open);
      open.disabled=["failed","cancelled"].includes(job.state);
      open.onclick=async()=>{if(job.metadata?.analysisId)await openAnalysis(job.metadata.analysisId);else{state.analysisId=null;state.jobId=job.id;if(job.state==="finished")await loadResult(job.id);else listenForProgress(job.id);}jobsDialog.close();};
      if(["queued","running"].includes(job.state)||job.input&&["failed","cancelled"].includes(job.state)){
        const operation=["queued","running"].includes(job.state)?"cancel":"resubmit";
        const button=document.createElement("button");button.textContent=operation==="cancel"?"Cancel":"Resubmit";row.append(" ",button);
        button.onclick=async()=>{try{await json(`/api/jobs/${job.id}/${operation}`,post({}));await showJobs();}catch(e){noticeDialog(e.message);}};
      }
      jobsDialog.append(row);
    }
    const actions=document.createElement("div");actions.className="dialog-actions";jobsDialog.append(actions);
    const refresh=document.createElement("button");refresh.textContent="Refresh";refresh.onclick=()=>showJobs().catch(e=>noticeDialog(e.message));actions.append(refresh);
    const close=document.createElement("button");close.textContent="Close";close.onclick=()=>jobsDialog.close();actions.append(close);
  }
  jobsButton.onclick=()=>{jobsDialog.showModal();showJobs().catch(e=>noticeDialog(e.message));};
  let presenceBusy=false;
  setInterval(async()=>{
    if(!state.analysisId||state.loadingAnalysis||presenceBusy){if(!state.analysisId)presence.textContent="";return;}
    presenceBusy=true;const aid=state.analysisId;
    try{const people=await json(`/api/analyses/${aid}/presence`,post({viewId,cluster:state.presenceCluster||(state.selection?.cluster==null?null:String(state.selection.cluster))}));
      if(aid===state.analysisId)presence.textContent=people.map(p=>`${p.user.name}${p.cluster?` (cluster ${p.cluster})`:""}`).join(" · ");
    }catch{}finally{presenceBusy=false;}
  },2000);
  setInterval(async()=>{
    const central=!!state.analysisId;document.getElementById("centralSave").disabled=central||!state.analysis;
    document.getElementById("sharedTitle").textContent=state.shared?.analysis.name|| (state.analysis?"Temporary analysis — not centrally saved":"");
    if(!central||state.loadingAnalysis||viewBusy)return;
    const aid=state.analysisId,view=captureView(state),signature=JSON.stringify([aid,view]);if(signature===previousView)return;
    viewBusy=true;const status=document.getElementById("personalSaveStatus");status.textContent="Saving view…";
    try{await json(`/api/analyses/${aid}/view`,post(view));previousView=signature;status.textContent="View saved";}
    catch{status.textContent="View not saved — retrying";}finally{viewBusy=false;}
  },750);
}
async function openAnalysis(aid){state.analysisId=aid;state.jobId=aid;state.saveStatus="Saved";await loadResult(aid);previousView=JSON.stringify([aid,captureView(state)]);}
