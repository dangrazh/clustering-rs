import { openReviewTarget } from "./target-navigation.js";
import {state} from "./state.js";
import {statusLabels} from "./workflow.js";
import {json,post,node,empty,cell,dateCell,count,head,openButton,moreButton,listPanel,replaceList,searchInput,checkbox,select} from "./list-ui.js";
// Quadrants in reading order: top left, top right, bottom left, bottom right.
const quadrants=[["assigned","My assigned work"],["mine","My analyses"],["assignedAll","All assigned work"],["all","All analyses"]];
export async function initializeDashboard(openAnalysis){
  const saved=await json("/api/dashboard/preferences");
  const prefs={version:1,mine:{search:"",archived:false,...saved.mine},all:{search:"",archived:false,...saved.all},assigned:{archived:false,status:"",...saved.assigned},assignedAll:{archived:false,status:"",assignee:"",...saved.assignedAll},expanded:Array.isArray(saved.expanded)?saved.expanded:[]};
  const sections={},grid=document.getElementById("dashboardGrid");let data=null,busy=false,again=false,dirty=false,saving=false,signature="",dataDirty=true;
  const isVisible=()=>document.getElementById("dashboard").classList.contains("active")&&!document.hidden;
  const saveStatus=document.getElementById("dashboardSave");
  function changed(){dirty=true;saveStatus.textContent="Saving dashboard settings…";render();}
  async function save(){if(!dirty||saving)return;saving=true;dirty=false;const value=structuredClone(prefs);try{await json("/api/dashboard/preferences",post(value));saveStatus.textContent=dirty?"Saving dashboard settings…":"Dashboard settings saved";}catch{dirty=true;saveStatus.textContent="Settings not saved — retrying";}finally{saving=false;}}
  const statusOptions=[["","All statuses"],...Object.entries(statusLabels)];
  for(const [key,title] of quadrants){
    const section=listPanel(key,title,"dashboard");grid.append(section.panel);sections[key]=section;const {controls}=section;
    if(key==="mine"||key==="all")controls.append(searchInput(`Search ${title}`,prefs[key].search,value=>{prefs[key].search=value;changed();}));
    if(key==="assignedAll"){const assignee=select("Assignee",[["","All assignees"]],prefs.assignedAll.assignee,value=>{prefs.assignedAll.assignee=value;changed();});assignee.dataset.role="assignee";controls.append(assignee);}
    if(key==="assigned"||key==="assignedAll")controls.append(select(`${title} status`,statusOptions,prefs[key].status,value=>{prefs[key].status=value;changed();}));
    controls.append(checkbox("Show archived",!!prefs[key].archived,value=>{prefs[key].archived=value;changed();}));
    empty(section.list,"Loading…");
  }
  const analysisLimits={mine:100,all:100};
  function analysisList(key){replaceList(sections[key].list,list=>{let entries=data.analyses.filter(a=>(key!=="mine"||a.owner.id===state.user.id)&&(prefs[key].archived||!a.archived)&&a.name.normalize().toLowerCase().includes(prefs[key].search.trim().normalize().toLowerCase()));let shown=0;const more=()=>{for(const a of entries.slice(shown,analysisLimits[key])){const row=node("div",null,"dashboard-row");row.append(openButton(a.name,`${key}:${a.id}`,()=>openAnalysis(a.id)),cell(a.owner.name),dateCell(new Date(a.created*1000)),cell(a.archived?"Archived":"Active"));list.append(row);}shown=analysisLimits[key];if(shown<entries.length){const next=moreButton("Show more",()=>{next.remove();analysisLimits[key]+=100;more();});list.append(next);}};if(!entries.length)empty(list,"No analyses match this view.");else{list.append(head("Name","Owner","Created","State"));more();}});}
  const openTarget=(aid,item)=>openReviewTarget(openAnalysis,aid,item.entity);
  const groupLimits={assigned:100,assignedAll:100},themeLimits=new Map();
  // Assigned clusters per analysis; a theme assigned on its own shows its cluster as context.
  function appendGroups(list,key,items,scope,counter){
    const filter=prefs[key],analyses=data.analyses.filter(a=>filter.archived||!a.archived).sort((a,b)=>a.name.localeCompare(b.name));
    const byAnalysis=new Map();for(const item of items){if(!byAnalysis.has(item.analysisId))byAnalysis.set(item.analysisId,[]);byAnalysis.get(item.analysisId).push(item);}
    let lastAnalysis=null;
    for(const a of analyses){
      const all=byAnalysis.get(a.id)||[],matching=all.filter(i=>!filter.status||i.status===filter.status);
      const parents=[...new Set(matching.map(i=>i.parent))].sort((x,y)=>Number(x)-Number(y));
      for(const parent of parents){
        if(++counter.groups>groupLimits[key])continue;
        if(counter.groups===1)list.append(head("Cluster / theme","Status","Incidents"));
        if(counter.scope!==scope){counter.scope=scope;if(scope)list.append(node("div",scope,"dashboard-assignee"));}
        if(lastAnalysis!==a.id){list.append(node("h4",a.name+(a.archived?" (Archived)":"")));lastAnalysis=a.id;}
        const child=all.find(i=>i.parent===parent),cluster=all.find(i=>i.entity===parent)||{...child,entity:parent,label:child.parentLabel,incidents:child.parentIncidents,contextOnly:true};
        const groupKey=`${key==="assignedAll"?`${child.assignee.id}:`:""}${a.id}:${parent}`,details=node("details"),summary=node("summary");
        summary.append(cell(`Cluster ${parent} — ${cluster.label}`,"dashboard-title"),cell(statusLabels[cluster.status]||cluster.status),count(cluster.incidents));
        summary.dataset.key=`group:${key}:${groupKey}`;details.open=prefs.expanded.includes(groupKey);details.append(summary);
        const children=node("div");details.append(children);let rendered=false;
        const row=(item,theme=false)=>{const el=node("div",null,`dashboard-row${theme?" dashboard-theme":""}`);el.append(openButton(`${theme?"Theme":"Cluster"} ${item.entity} — ${item.label}`,`${key}:${groupKey}:${item.entity}`,()=>openTarget(a.id,item)),cell(statusLabels[item.status]||item.status),count(item.incidents));return el;};
        function renderChildren(){
          rendered=true;children.replaceChildren();
          if(!cluster.contextOnly&&(!filter.status||cluster.status===filter.status))children.append(row(cluster));
          else children.append(node("p","Parent context — assigned work is listed below","dashboard-meta"));
          const themes=matching.filter(i=>i.parent===parent&&i.entity!==parent).sort((x,y)=>Number(x.entity.split(":")[1])-Number(y.entity.split(":")[1]));
          const limit=themeLimits.get(groupKey)||100;
          for(const theme of themes.slice(0,limit))children.append(row(theme,true));
          if(themes.length>limit)children.append(moreButton("Show more themes",()=>{themeLimits.set(groupKey,limit+100);renderChildren();}));
        }
        if(details.open)renderChildren();
        details.ontoggle=()=>{
          if(details.open&&!rendered)renderChildren();
          const has=prefs.expanded.includes(groupKey);if(details.open===has)return;
          prefs.expanded=details.open?[...prefs.expanded,groupKey].slice(-1000):prefs.expanded.filter(k=>k!==groupKey);
          dirty=true;saveStatus.textContent="Saving dashboard settings…";
        };
        list.append(details);
      }
    }
  }
  function assignedList(key){
    replaceList(sections[key].list,list=>{
      const counter={groups:0,scope:null};
      if(key==="assigned")appendGroups(list,key,data.assigned,null,counter);
      else{
        // Group by assignee; the filter narrows to one person.
        const people=new Map();for(const item of data.assignedAll)people.set(item.assignee.id,item.assignee.name);
        const chooser=sections.assignedAll.controls.querySelector('[data-role="assignee"]');
        chooser.replaceChildren(new Option("All assignees",""),...[...people].sort((a,b)=>a[1].localeCompare(b[1])).map(([id,name])=>new Option(name,id)));
        chooser.value=people.has(prefs.assignedAll.assignee)?prefs.assignedAll.assignee:"";
        for(const [id,name] of [...people].sort((a,b)=>a[1].localeCompare(b[1]))){
          if(chooser.value&&id!==chooser.value)continue;
          appendGroups(list,key,data.assignedAll.filter(i=>i.assignee.id===id),name,counter);
        }
      }
      if(!counter.groups)empty(list,"No assigned work matches this view.");
      if(counter.groups>groupLimits[key])list.append(moreButton("Show more assigned clusters",()=>{groupLimits[key]+=100;assignedList(key);}));
    });
  }
  async function fetchData(){
    const result={analyses:[],assigned:[],assignedAll:[]};let offset=0;
    do{const page=await json(`/api/dashboard/data?offset=${offset}`);result.analyses.push(...page.analyses);result.assigned.push(...page.assigned);offset=page.nextOffset;}while(offset!=null);
    offset=0;do{const page=await json(`/api/dashboard/assigned-all?offset=${offset}`);result.assignedAll.push(...page.assigned);offset=page.nextOffset;}while(offset!=null);
    result.analyses=[...new Map(result.analyses.map(a=>[a.id,a])).values()];
    result.assigned=[...new Map(result.assigned.map(a=>[`${a.analysisId}:${a.entity}`,a])).values()];
    result.assignedAll=[...new Map(result.assignedAll.map(a=>[`${a.assignee.id}:${a.analysisId}:${a.entity}`,a])).values()];
    return result;
  }
  function render(){if(data){assignedList("assigned");analysisList("mine");assignedList("assignedAll");analysisList("all");}}
  async function refresh(){if(!isVisible())return;if(busy){again=true;return;}busy=true;try{do{again=false;if(!dataDirty)break;dataDirty=false;try{const value=await fetchData();for(const [k] of quadrants)sections[k].error.textContent="";const next=JSON.stringify(value);if(next!==signature){data=value;signature=next;render();}}catch(e){dataDirty=true;for(const [k] of quadrants)sections[k].error.textContent=`Unable to refresh: ${e.message}`;}}while(again&&isVisible());}finally{busy=false;}}
  window.addEventListener("dashboard-refresh",()=>{dataDirty=true;refresh();});window.addEventListener("pane-change",e=>{if(e.detail==="dashboard"){dataDirty=true;setTimeout(refresh,0);}});document.addEventListener("visibilitychange",()=>{dataDirty=true;refresh();});
  setInterval(save,750);await refresh();
}
