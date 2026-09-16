import {jobSummaryText,jobAction} from "./jobs-view.js";
import {state} from "./state.js";
import {request} from "./api.js";
import {loadResult,renderResults,treeVisibleRows} from "./results.js";
import {listenForProgress} from "./analysis.js";
import {noticeDialog} from "./ui.js";
import {statusLabels,effectiveWorkflow} from "./workflow.js";
const post=value=>({method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(value)});
async function json(url,options){const r=await request(url,options),v=await r.json();if(!r.ok)throw Error(v.error||r.statusText);return v;}
const node=(tag,text,cls)=>{const e=document.createElement(tag);if(text!=null)e.textContent=text;if(cls)e.className=cls;return e;};
const button=(text,action)=>{const b=node("button",text);b.type="button";b.onclick=()=>Promise.resolve().then(action).catch(e=>noticeDialog(e.message));return b;};
const empty=(list,text)=>list.append(node("p",text,"dashboard-empty"));
export async function initializeDashboard(openAnalysis){
  const saved=await json("/api/dashboard/preferences");
  const prefs={version:1,mine:{search:"",archived:false,...saved.mine},all:{search:"",archived:false,...saved.all},assigned:{archived:false,status:"",...saved.assigned},expanded:Array.isArray(saved.expanded)?saved.expanded:[]};
  const sections={},grid=document.getElementById("dashboardGrid");let data=null,jobs=null,busy=false,again=false,dirty=false,saving=false,signature="",jobsSignature="",dataDirty=true;
  const isVisible=()=>document.getElementById("dashboard").classList.contains("active")&&!document.hidden;
  const saveStatus=document.getElementById("dashboardSave");
  function changed(){dirty=true;saveStatus.textContent="Saving dashboard settings…";render();}
  async function save(){if(!dirty||saving)return;saving=true;dirty=false;const value=structuredClone(prefs);try{await json("/api/dashboard/preferences",post(value));saveStatus.textContent=dirty?"Saving dashboard settings…":"Dashboard settings saved";}catch{dirty=true;saveStatus.textContent="Settings not saved — retrying";}finally{saving=false;}}
  for(const [key,title] of [["jobs","My jobs"],["mine","My analyses"],["assigned","Assigned work"],["all","All analyses"]]){
    const panel=node("section",null,"dashboard-panel"),heading=node("h3",title),controls=node("div",null,"dashboard-controls"),error=node("p",null,"dashboard-error"),list=node("div",null,"dashboard-list");heading.id=`dashboard-${key}-title`;panel.setAttribute("aria-labelledby",heading.id);error.setAttribute("role","status");panel.append(heading,controls,error,list);grid.append(panel);sections[key]={panel,list,error};
    if(key==="mine"||key==="all"){const input=node("input");input.type="search";input.placeholder="Search by name";input.setAttribute("aria-label",`Search ${title}`);input.value=prefs[key].search;input.oninput=()=>{prefs[key].search=input.value.slice(0,200);changed();};controls.append(input);}
    if(key!=="jobs"){const label=node("label"),input=node("input");input.type="checkbox";input.checked=!!prefs[key].archived;input.onchange=()=>{prefs[key].archived=input.checked;changed();};label.append(input,document.createTextNode(" Show archived"));controls.append(label);}
    if(key==="assigned"){const select=node("select");select.setAttribute("aria-label","Assigned work status");for(const [value,label] of [["","All statuses"],...Object.entries(statusLabels)]){const o=node("option",label);o.value=value;select.append(o);}select.value=prefs.assigned.status;select.onchange=()=>{prefs.assigned.status=select.value;changed();};controls.append(select);}
    empty(list,"Loading…");
  }
  function replaceList(key,build){const list=sections[key].list,scroll=list.scrollTop;const focus=document.activeElement?.dataset?.key;list.replaceChildren();build(list);list.scrollTop=scroll;if(focus){const match=[...list.querySelectorAll("[data-key]")].find(e=>e.dataset.key===focus);match?.focus({preventScroll:true});}}
  const analysisLimits={mine:100,all:100};
  function analysisList(key){replaceList(key,list=>{let entries=data.analyses.filter(a=>(key!=="mine"||a.owner.id===state.user.id)&&(prefs[key].archived||!a.archived)&&a.name.normalize().toLowerCase().includes(prefs[key].search.trim().normalize().toLowerCase()));let shown=0;const more=()=>{for(const a of entries.slice(shown,analysisLimits[key])){const row=node("div",null,"dashboard-row"),open=button(a.name,()=>openAnalysis(a.id));open.dataset.key=`${key}:${a.id}`;row.append(open,node("p",`${a.owner.name} · ${new Date(a.created*1000).toLocaleString()} · ${a.archived?"Archived":"Active"}`,"dashboard-meta"));list.append(row);}shown=analysisLimits[key];if(shown<entries.length){const next=button("Show more",()=>{next.remove();analysisLimits[key]+=100;more();});list.append(next);}};if(!entries.length)empty(list,"No analyses match this view.");else more();});}
  async function openTarget(aid,item){if(await openAnalysis(aid)===false)return;if(state.analysisId!==aid)return;const [clusterId,themeId]=item.entity.split(":").map(Number);const cluster=state.analysis.clusters.find(c=>c.id===clusterId),theme=themeId==null?null:cluster?.subgroups.find(t=>t.id===themeId);if(!cluster||themeId!=null&&!theme)throw Error("This review item is no longer available.");state.selection=theme?{type:"theme",cluster:clusterId,theme:themeId}:{type:"cluster",cluster:clusterId};state.expandedClusters.add(String(clusterId));const visible=treeVisibleRows();if(visible&&!(theme||cluster).incident_row_indices.some(row=>visible.has(row))){state.workflowStates=[];state.reviewerFilter={};state.detailColumnFilters=[];state.detailDrilldownRowIndices=null;state.detailDrilldownLabel="";}if(theme&&state.workflowStates.length&&!state.workflowStates.includes(effectiveWorkflow(state.review,item.entity).status))state.workflowStates=[];state.detailPage=1;renderResults();}
  let assignedLimit=100;
  const themeLimits=new Map();
  function assignedList(){
    replaceList("assigned",list=>{
      const analyses=data.analyses.filter(a=>prefs.assigned.archived||!a.archived).sort((a,b)=>a.name.localeCompare(b.name));
      const byAnalysis=new Map();
      for(const item of data.assigned){if(!byAnalysis.has(item.analysisId))byAnalysis.set(item.analysisId,[]);byAnalysis.get(item.analysisId).push(item);}
      let groups=0,lastAnalysis=null;
      for(const a of analyses){
        const items=byAnalysis.get(a.id)||[];
        const matching=items.filter(i=>!prefs.assigned.status||i.status===prefs.assigned.status);
        const parents=[...new Set(matching.map(i=>i.parent))].sort((x,y)=>Number(x)-Number(y));
        for(const parent of parents){
          if(++groups>assignedLimit)continue;
          const cluster=items.find(i=>i.entity===parent);if(!cluster)continue;
          if(lastAnalysis!==a.id){list.append(node("h4",a.name+(a.archived?" (Archived)":"")));lastAnalysis=a.id;}
          const key=`${a.id}:${parent}`,details=node("details"),summary=node("summary",`Cluster ${parent} — ${cluster.label}`);
          summary.append(node("span",` · ${statusLabels[cluster.status]||cluster.status} · ${cluster.incidents.toLocaleString()} incidents`,"dashboard-meta"));
          summary.dataset.key=`group:${key}`;details.open=prefs.expanded.includes(key);details.append(summary);
          const children=node("div");details.append(children);let rendered=false;
          const row=(item,theme=false)=>{
            const el=node("div",null,`dashboard-row${theme?" dashboard-theme":""}`);
            const open=button(`${theme?"Theme":"Cluster"} ${item.entity} — ${item.label}`,()=>openTarget(a.id,item));
            open.dataset.key=`assigned:${a.id}:${item.entity}`;
            el.append(open,node("p",`${statusLabels[item.status]||item.status} · ${item.incidents.toLocaleString()} incidents`,"dashboard-meta"));return el;
          };
          function renderChildren(){
            rendered=true;children.replaceChildren();
            if(!prefs.assigned.status||cluster.status===prefs.assigned.status)children.append(row(cluster));
            else children.append(node("p","Parent context — status does not match filter","dashboard-meta"));
            const themes=matching.filter(i=>i.parent===parent&&i.entity!==parent).sort((x,y)=>Number(x.entity.split(":")[1])-Number(y.entity.split(":")[1]));
            const limit=themeLimits.get(key)||100;
            for(const theme of themes.slice(0,limit))children.append(row(theme,true));
            if(themes.length>limit)children.append(button("Show more themes",()=>{themeLimits.set(key,limit+100);renderChildren();}));
          }
          if(details.open)renderChildren();
          details.ontoggle=()=>{
            if(details.open&&!rendered)renderChildren();
            const has=prefs.expanded.includes(key);if(details.open===has)return;
            prefs.expanded=details.open?[...prefs.expanded,key].slice(-1000):prefs.expanded.filter(k=>k!==key);
            dirty=true;saveStatus.textContent="Saving dashboard settings…";
          };
          list.append(details);
        }
      }
      if(!groups)empty(list,"No assigned work matches this view.");
      if(groups>assignedLimit)list.append(button("Show more assigned clusters",()=>{assignedLimit+=100;assignedList();}));
    });
  }
  async function openJob(original){const latest=(await json("/api/jobs")).find(j=>j.id===original.id);if(!latest)throw Error("This job is no longer available.");if(latest.metadata?.analysisId)return openAnalysis(latest.metadata.analysisId);if(latest.state==="finished"){state.analysisId=null;return loadResult(latest.id);}if(["queued","running"].includes(latest.state))return listenForProgress(latest.id);await noticeDialog(latest.error||`Job ${latest.state}. Use Resubmit to run it again.`);}
  function jobsList(){replaceList("jobs",list=>{if(!jobs.length)return empty(list,"You have no retained jobs.");for(const job of jobs){const row=node("div",null,"dashboard-row");const open=button(`${new Date(job.created).toLocaleString()} — ${job.state}`,()=>openJob(job));open.dataset.key=`job:${job.id}`;row.append(open,node("p",jobSummaryText(job),"dashboard-meta"));if(job.expires)row.append(node("p",`Available until ${new Date(job.expires*1000).toLocaleString()}`,"dashboard-meta"));if(job.error)row.append(node("p",job.error,"dashboard-error"));const operation=jobAction(job);if(operation){const action=button(operation==="cancel"?"Cancel":"Resubmit",async()=>{await json(`/api/jobs/${job.id}/${operation}`,post({}));await refresh();});if(operation==="resubmit"&&!job.input){action.disabled=true;action.title="The original input is no longer available.";}row.append(action);}list.append(row);}});}
  async function fetchData(){
    const result={analyses:[],assigned:[]};let offset=0;
    do{const page=await json(`/api/dashboard/data?offset=${offset}`);result.analyses.push(...page.analyses);result.assigned.push(...page.assigned);offset=page.nextOffset;}while(offset!=null);
    result.analyses=[...new Map(result.analyses.map(a=>[a.id,a])).values()];
    result.assigned=[...new Map(result.assigned.map(a=>[`${a.analysisId}:${a.entity}`,a])).values()];
    return result;
  }
  function render(){if(data){analysisList("mine");analysisList("all");assignedList();}}
  async function refresh(){if(!isVisible())return;if(busy){again=true;return;}busy=true;try{do{again=false;await Promise.all([(dataDirty?(dataDirty=false,fetchData()):Promise.resolve(null)).then(value=>{if(!value)return;for(const k of ["mine","all","assigned"])sections[k].error.textContent="";const next=JSON.stringify(value);if(next!==signature){data=value;signature=next;render();}}).catch(e=>{dataDirty=true;for(const k of ["mine","all","assigned"])sections[k].error.textContent=`Unable to refresh: ${e.message}`;}),json("/api/jobs").then(value=>{sections.jobs.error.textContent="";const next=JSON.stringify(value);if(next!==jobsSignature){jobs=value;jobsSignature=next;jobsList();}}).catch(e=>{sections.jobs.error.textContent=`Unable to refresh: ${e.message}`;})]);}while(again&&isVisible());}finally{busy=false;}}
  window.addEventListener("dashboard-refresh",()=>{dataDirty=true;refresh();});window.addEventListener("pane-change",e=>{if(e.detail==="dashboard"){dataDirty=true;setTimeout(refresh,0);}});document.addEventListener("visibilitychange",()=>{dataDirty=true;refresh();});
  setInterval(refresh,3000);setInterval(save,750);await refresh();
}
