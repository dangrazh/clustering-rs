// "Analyses and Jobs": every saved analysis beside the jobs list, as dense tables.
import {state} from "./state.js";
import {loadResult} from "./results.js";
import {listenForProgress} from "./analysis.js";
import {askDialog,noticeDialog,showStep} from "./ui.js";
import {jobSummaryText,jobAction} from "./jobs-view.js";
import {json,post,node,empty,cell,dateCell,shortDate,head,openButton,moreButton,actionButton,listPanel,replaceList,searchInput,checkbox} from "./list-ui.js";

const readOnly=()=>state.role==="Read Only User";
// The open analysis was deleted: drop it so nothing can be saved back. Viewers of its
// results return to the Dashboard and are told; the person deleting it stays where they are.
export function closeDeletedAnalysis(name,{quiet=false}={}){
  Object.assign(state,{analysisId:null,jobId:null,analysis:null,shared:null,selection:null});
  for(const step of ["results","pivot"])document.querySelector(`[data-step="${step}"]`).disabled=true;
  if(document.querySelector("#results.active, #pivot.active"))showStep("dashboard");
  if(!quiet)noticeDialog(`The analysis “${name||"you had open"}” was permanently deleted.`);
}
const admin=()=>state.role==="SysAdmin";
export function initializeCatalog(openAnalysis){
  const grid=document.getElementById("catalogGrid");if(!grid)return;
  const analyses=listPanel("catalogAnalyses","Analyses"),jobs=listPanel("catalogJobs","Jobs");grid.append(analyses.panel,jobs.panel);
  const filter={search:"",archived:false},jobFilter={all:true};let data=null,jobData=null,limit=200,busy=false,again=false;
  analyses.controls.append(searchInput("Search analyses","",value=>{filter.search=value;renderAnalyses();}),checkbox("Show archived",false,value=>{filter.archived=value;renderAnalyses();}));
  const isVisible=()=>document.getElementById("catalog").classList.contains("active")&&!document.hidden;
  function command(a,type,extra={}){return json(`/api/analyses/${a.id}/commands`,post({commandId:crypto.randomUUID(),target:"",expected:a.version,action:{type,...extra}}));}
  async function rename(a){const name=await askDialog("Rename analysis","Choose a unique analysis name.",{value:a.name,confirmLabel:"Rename"});if(!name||name===a.name)return;await command(a,"renameAnalysis",{name});await refresh();}
  async function archive(a){
    if(a.archived)await command(a,"restore");
    else{if(!await askDialog("Archive analysis",`Archive “${a.name}”? Everyone reviewing it will be notified and editing stops until it is restored.`,{confirmLabel:"Archive"}))return;await command(a,"archive");}
    await refresh();
  }
  async function remove(a){
    const confirmed=await askDialog("Permanently delete analysis",`Permanently delete “${a.name}”? Its clusters, review statuses, assignments, comments, history and everyone's saved views are removed. This cannot be undone.`,{value:"",label:`Type the analysis name to confirm`,confirmLabel:"Permanently delete"});
    if(confirmed==null)return;
    if(confirmed.trim()!==a.name.trim())return noticeDialog("The name did not match. Nothing was deleted.");
    await json(`/api/analyses/${a.id}/delete`,post({commandId:crypto.randomUUID(),expected:a.version}));
    if(state.analysisId===a.id)closeDeletedAnalysis(a.name,{quiet:true});
    await refresh();
  }
  function renderAnalyses(){
    if(!data)return;
    replaceList(analyses.list,list=>{
      const query=filter.search.trim().normalize().toLowerCase();
      const entries=data.filter(a=>(filter.archived||!a.archived)&&a.name.normalize().toLowerCase().includes(query));
      if(!entries.length)return empty(list,"No analyses match this view.");
      list.append(head("Name","Owner","Created","State",""));
      for(const a of entries.slice(0,limit)){
        const row=node("div",null,"dashboard-row"),actions=node("span",null,"dashboard-actions");
        const owner=a.owner.id===state.user?.id,deletable=!readOnly()&&(owner||admin());
        const renameButton=actionButton("Rename",()=>rename(a)),archiveButton=actionButton(a.archived?"Restore":"Archive",()=>archive(a)),deleteButton=actionButton("Delete",()=>remove(a));
        renameButton.disabled=readOnly()||a.archived;archiveButton.disabled=readOnly();deleteButton.disabled=!deletable;deleteButton.classList.add("danger");
        if(a.archived&&!readOnly())renameButton.title="Restore the analysis to rename it.";
        if(!deletable&&!readOnly())deleteButton.title="Only the owner or a SysAdmin can delete this analysis.";
        actions.append(renameButton,archiveButton,deleteButton);
        row.append(openButton(a.name,`analysis:${a.id}`,()=>openAnalysis(a.id)),cell(a.owner.name),dateCell(new Date(a.created*1000)),cell(a.archived?"Archived":"Active"),actions);list.append(row);
      }
      if(entries.length>limit)list.append(moreButton("Show more",()=>{limit+=200;renderAnalyses();}));
    });
  }
  async function openJob(original){const latest=jobData.find(j=>j.id===original.id);if(!latest)throw Error("This job is no longer available.");if(latest.metadata?.analysisId)return openAnalysis(latest.metadata.analysisId);if(latest.state==="finished"){state.analysisId=null;return loadResult(latest.id);}if(["queued","running"].includes(latest.state))return listenForProgress(latest.id);await noticeDialog(latest.error||`Job ${latest.state}. Use Resubmit to run it again.`);}
  function renderJobs(){
    if(!jobData)return;
    const everyone=admin()&&jobFilter.all;jobs.list.dataset.scope=everyone?"all":"mine";
    replaceList(jobs.list,list=>{
      if(!jobData.length)return empty(list,everyone?"There are no retained jobs.":"You have no retained jobs.");
      list.append(everyone?head("Created","Owner","State","Details","Available until",""):head("Created","State","Details","Available until",""));
      for(const job of jobData){
        const row=node("div",null,"dashboard-row"),created=new Date(job.created),own=!everyone||job.owner===state.user?.id;
        // Other users' jobs hold their uploads, so operators see and cancel them but cannot open them.
        const open=own?openButton(shortDate(created),`job:${job.id}`,()=>openJob(job)):cell(shortDate(created),"dashboard-meta",created.toLocaleString());
        row.append(open);if(everyone)row.append(cell(job.ownerUser?.name||"Unknown"));
        row.append(cell(job.state),cell(jobSummaryText(job)),job.expires?dateCell(new Date(job.expires*1000)):cell("—"));
        const operation=own?jobAction(job):(["queued","running"].includes(job.state)?"cancel":null);
        if(!operation)row.append(node("span"));
        else{const action=actionButton(operation==="cancel"?"Cancel":"Resubmit",async()=>{await json(`/api/jobs/${job.id}/${operation}`,post({}));await refresh();});if(operation==="resubmit"&&!job.input){action.disabled=true;action.title="The original input is no longer available.";}row.append(action);}
        if(job.error)row.append(cell(job.error,"dashboard-error"));
        list.append(row);
      }
    });
  }
  async function fetchAnalyses(){let all=[],offset=0;do{const page=await json(`/api/dashboard/data?offset=${offset}`);all.push(...page.analyses);offset=page.nextOffset;}while(offset!=null);return[...new Map(all.map(a=>[a.id,a])).values()];}
  async function refresh(){
    if(!isVisible())return;if(busy){again=true;return;}busy=true;
    try{do{again=false;await Promise.all([
      fetchAnalyses().then(value=>{analyses.error.textContent="";data=value;renderAnalyses();}).catch(e=>{analyses.error.textContent=`Unable to refresh: ${e.message}`;}),
      json(`/api/jobs${admin()&&jobFilter.all?"?scope=all":""}`).then(value=>{jobs.error.textContent="";jobData=value;renderJobs();}).catch(e=>{jobs.error.textContent=`Unable to refresh: ${e.message}`;}),
    ]);}while(again);}finally{busy=false;}
  }
  // The role is known only after sign-in; operators can switch between everyone's jobs and their own.
  window.addEventListener("pane-change",e=>{
    if(e.detail!=="catalog")return;
    if(admin()&&!jobs.controls.childElementCount)jobs.controls.append(checkbox("Show all users' jobs",jobFilter.all,value=>{jobFilter.all=value;jobData=null;refresh();}));
    setTimeout(refresh,0);
  });
  window.addEventListener("dashboard-refresh",()=>refresh());
  setInterval(refresh,3000);
}
