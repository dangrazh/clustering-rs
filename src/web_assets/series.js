import {request} from "./api.js";
import {askDialog,noticeDialog,showStep} from "./ui.js";
import {openReviewTarget} from "./target-navigation.js";
import {renderSeriesChart} from "./series-chart.js";

const el=(tag,text,cls)=>{const n=document.createElement(tag);if(text!=null)n.textContent=text;if(cls)n.className=cls;return n;};
const post=value=>({method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(value)});
async function api(url,options){const r=await request(url,options),v=await r.json();if(!r.ok)throw Error(v.error||r.statusText);return v;}
function button(text,action){const b=el("button",text);b.type="button";b.onclick=async()=>{b.disabled=true;try{await action();}catch(e){await noticeDialog(e.message);}finally{if(b.isConnected)b.disabled=false;}};return b;}
function field(label,input){const n=el("div",null,"field"),caption=el("label",label);input.id||=`series-field-${crypto.randomUUID()}`;caption.htmlFor=input.id;n.append(caption,input);return n;}
function input(type,value=""){const n=el("input");n.type=type;n.value=value;return n;}
function select(options,value){const n=el("select");for(const [id,label]of options){const o=el("option",label);o.value=id;n.append(o);}if(value!=null)n.value=value;return n;}
function modal(title,build,{confirmLabel="Continue",cancel=true}={}){return new Promise(resolve=>{
  const dialog=el("dialog",null,"app-dialog series-dialog"),form=el("form"),heading=el("h2",title);heading.id=`series-${crypto.randomUUID()}`;dialog.setAttribute("aria-labelledby",heading.id);
  const content=el("div",null,"series-dialog-body"),actions=el("div",null,"dialog-actions"),error=el("p",null,"dashboard-error");error.setAttribute("role","alert");
  const cancelButton=button("Cancel",()=>dialog.close()),save=el("button",confirmLabel,"primary");save.type="submit";if(cancel)actions.append(cancelButton);actions.append(save);form.append(content,error,actions);dialog.append(heading,form);document.body.append(dialog);
  const read=build(content);let result=null;
  form.onsubmit=e=>{e.preventDefault();try{result=read();dialog.close();}catch(e){error.textContent=e.message;}};
  dialog.addEventListener("close",()=>{dialog.remove();resolve(result);},{once:true});dialog.showModal();(content.querySelector("input,select")||save).focus();
});}
function prepareAnalysis(body){
  document.querySelectorAll("dialog.series-dialog[open]").forEach(dialog=>dialog.close());showStep("source");
  document.getElementById("source")?.scrollIntoView({block:"start"});
}
async function showSeriesGuide(){
  await modal("How to create a series",body=>{
    body.append(el("p","A series compares clusters from analyses of separate periods, for example January and February. Start with one analysis per period; two periods let you compare counts."));
    const steps=el("ol",null,"series-guide-steps");
    for(const [title,text] of [
      ["Prepare and save each analysis","For a CSV/XLSX file: go to Source → Open CSV/XLSX, map the fields, and run clustering. For an existing .icas file: go to Source → Load Session; no new clustering run is needed. Once results are available, click Save centrally in the top bar and give the analysis a name. Repeat for the next period."],
      ["Create or open a series","Return to Series and click New series, or choose an existing series. Give it a name such as Monthly payment issues. You do not need to open an analysis in Results before adding it."],
      ["Add a saved analysis and its dates","Click Add saved analysis, search by name, and choose the centrally saved analysis. Enter the inclusive start and end dates of its period. Use non-overlapping dates and one analysis per period. Click Add and match. The dates describe the data; they do not filter records."],
      ["Match and view trends","The wizard calculates and saves strong matches automatically. Review uncertain matches in the next step or leave them pending for later. Click View updated chart to finish. Add subsequent periods to the same series. Replacements and historical changes still show their impact before saving."],
      ["Review the trends","The first period establishes tracked problems. For later periods, open Cluster associations → Review link to confirm which clusters represent the same problem. Strong, unambiguous matches are automatic; uncertain matches need your confirmation. No matched cluster means unknown, not zero. Click a chart point or count to inspect its source clusters."]
    ]){const step=el("li");step.append(el("strong",title),el("p",text));steps.append(step);}
    body.append(steps,el("p","Cannot find an analysis? Only centrally saved analyses appear. A local .icas file or an unsaved job result must be saved centrally first. An analysis can belong to only one series. Use Periods → Replace for a rerun of an existing period."),button("Go to Source to prepare or import",()=>prepareAnalysis(body)));
    return()=>true;
  },{confirmLabel:"Close",cancel:false});
}
export function changes(before,after){
  const list=[];if(JSON.stringify(before.model)!==JSON.stringify(after.model))list.push(`Matching model: ${after.model?.version||"none"}; validation: ${after.model?.validation||"not yet accepted"}.`);const names=new Map([...before.problems,...after.problems].map(p=>[p.id,p.name]));
  const association=o=>o?.problem?(names.get(o.problem)||o.problem):"Pending / no link";
  for(const p of after.periods){const old=before.periods.find(v=>v.id===p.id);
    if(!old)list.push(`Add ${p.name}: ${p.start} – ${p.end}`);
    if(old&&old.analysis!==p.analysis)list.push(`Replace ${old.name} with ${p.name}; old source links leave current totals.`);
    for(const o of p.occurrences){const prev=old?.analysis===p.analysis?old.occurrences.find(v=>v.cluster===o.cluster):null;
      if(!prev||prev.problem!==o.problem||prev.manual!==o.manual||prev.reason!==o.reason||JSON.stringify(prev.rejected)!==JSON.stringify(o.rejected))list.push(`${p.start} · Cluster ${o.cluster}: ${association(prev)} → ${association(o)} (${o.count} records). ${o.reason}${o.manual?"; protected manual decision":""}`);
    }
  }
  for(const p of before.periods){const next=after.periods.find(v=>v.id===p.id&&v.analysis===p.analysis);if(!next)for(const o of p.occurrences.filter(o=>o.problem))list.push(`Remove old source link: ${p.start} · Cluster ${o.cluster} → ${association(o)} (${o.count} records). History is retained.`);}
  for(const z of after.zeros)if(!before.zeros.some(b=>b.period===z.period&&b.problem===z.problem))list.push(`Confirm zero: ${names.get(z.problem)} · ${after.periods.find(p=>p.id===z.period)?.start}`);
  for(const z of before.zeros)if(!after.zeros.some(b=>b.period===z.period&&b.problem===z.problem))list.push(`Revoke zero: ${names.get(z.problem)} · ${before.periods.find(p=>p.id===z.period)?.start}`);
  return list;
}
export function initializeSeries(openAnalysis){
  const root=document.getElementById("seriesRoot");if(!root)return;
  let current=null,active=false,busy=false,again=false,generation=0,rows=[],selected=new Set(),plotInitialized=false,percentage=false;
  let search="",archived=false,catalogOffset=null,taskTimer=null;
  const catalog=el("div"),detail=el("div"),message=el("p",null,"job-metadata");message.setAttribute("role","status");const intro=el("div",null,"series-start dashboard-panel");
  intro.append(el("h3","Compare saved analyses across periods"),el("p","1. Save each analysis centrally.  2. Create or open a series.  3. Add a saved analysis, enter its dates, then follow the matching wizard."));
  const helpActions=el("div",null,"toolbar");helpActions.append(button("How to create a series",showSeriesGuide),button("Prepare or import an analysis",()=>prepareAnalysis()));intro.append(helpActions);
  root.append(intro,catalog,message,detail);
  const catalogBar=el("div",null,"toolbar"),searchInput=input("search"),archiveInput=input("checkbox");searchInput.placeholder="Search series";searchInput.setAttribute("aria-label","Search series");
  catalogBar.append(field("Search",searchInput),field("Show archived",archiveInput),button("New series",async()=>{const name=await askDialog("New series","Name the series, then choose the first centrally saved analysis to add. For example: Monthly payment issues.",{value:"",label:"Series name",confirmLabel:"Create series"});if(!name)return;const s=await api("/api/series",post({commandId:crypto.randomUUID(),name}));await open(s.id);await refreshCatalog();await addPeriod();}));
  const catalogList=el("div",null,"series-catalog");catalog.append(catalogBar,catalogList);
  let debounce;searchInput.oninput=()=>{search=searchInput.value;clearTimeout(debounce);debounce=setTimeout(()=>refreshCatalog().catch(report),250);};archiveInput.onchange=()=>{archived=archiveInput.checked;refreshCatalog().catch(report);};
  function report(e){message.textContent=e.message;}
  async function refreshCatalog(append=false){const token=search+archived;const data=await api(`/api/series?search=${encodeURIComponent(search)}&archived=${archived}&offset=${append?catalogOffset||0:0}`);if(token!==search+archived)return;if(!append)catalogList.replaceChildren();catalogList.querySelector(".series-more")?.remove();for(const s of data.items){const b=button(`${s.name}${s.archived?" · Archived":""}`,()=>open(s.id));b.classList.toggle("primary",s.id===current?.id);catalogList.append(b);}catalogOffset=data.nextOffset;if(catalogOffset!=null){const more=button("More series",()=>refreshCatalog(true));more.className="series-more";catalogList.append(more);}if(!catalogList.childElementCount)catalogList.append(el("p","No series match this view.","dashboard-empty"));}
  async function open(id){generation++;selected=new Set();plotInitialized=false;current={id,revision:-1};await refresh();}
  function deletedSeries(sid){
    if(current?.id!==sid)return;
    generation++;current=null;rows=[];selected.clear();clearTimeout(taskTimer);detail.replaceChildren();
    document.querySelectorAll("dialog.series-dialog[open]").forEach(d=>d.close());
    message.textContent="Series permanently deleted. Its saved analyses are available to add to another series.";
  }
  async function command(action,expected=current?.revision,sid=current?.id){if(!current)return;if(sid!==current.id||expected!==current.revision)throw Error("Series changed while you were editing. Review the latest state and retry.");const result=await api(`/api/series/${current.id}/commands`,post({commandId:crypto.randomUUID(),expected,action}));if(result.task)message.textContent="Preparing comparison. When ready, choose Review proposal, then Save to series. Nothing is added until you confirm.";await refresh();await refreshCatalog();return result;}
  async function refresh(){
    if(!current)return;if(busy){again=true;return;}busy=true;const sid=current.id,token=generation;
    try{
      const [s,tasks]=await Promise.all([api(`/api/series/${sid}`),api(`/api/series/${sid}/tasks`)]);let collected=[],offset=0;
      do{const page=await api(`/api/series/${sid}/trends?offset=${offset}`);if(page.revision!==s.revision){again=true;return;}collected.push(...page.rows);offset=page.nextOffset;}while(offset!=null);
      if(token!==generation||sid!==current?.id)return;
      const scroll=detail.querySelector(".series-table-wrap")?.scrollLeft||0;
      const expanded=new Set([...detail.querySelectorAll("details[open][data-key]")].map(n=>n.dataset.key));
      current=s;rows=collected;if(!plotInitialized&&rows.length){selected=new Set(rows.slice(0,6).map(r=>r.problem.id));plotInitialized=true;}
      render(tasks);for(const d of detail.querySelectorAll("details[data-key]"))d.open=expanded.has(d.dataset.key);
      const table=detail.querySelector(".series-table-wrap");if(table)table.scrollLeft=scroll;
      clearTimeout(taskTimer);if(active&&tasks.some(t=>["running","queued"].includes(t.state)))taskTimer=setTimeout(()=>refresh().catch(report),1500);
    }finally{busy=false;if(again){again=false;queueMicrotask(()=>refresh().catch(report));}}
  }
  async function addPeriod(replace){
    const base=current.revision,sid=current.id;
    const memberships=await api("/api/series/memberships"),membership=new Map(memberships.map(m=>[m.analysis,m]));
    let analyses=[],offset=0;do{const page=await api(`/api/dashboard/data?offset=${offset}`);analyses.push(...page.analyses);offset=page.nextOffset;}while(offset!=null);
    analyses=[...new Map(analyses.map(a=>[a.id,a])).values()].sort((a,b)=>a.name.localeCompare(b.name));
    if(!analyses.length)return modal("Save an analysis first",body=>{body.append(el("p","This series is ready, but there are no centrally saved analyses to add yet."),el("p","Go to Source to open a CSV/XLSX file and run clustering, or use Load Session for an existing .icas file. Then click Save centrally in the top bar. Return to this series and choose Add saved analysis."),button("Go to Source to prepare or import",()=>prepareAnalysis(body)),button("How to create a series",showSeriesGuide));return()=>true;},{confirmLabel:"Back to series",cancel:false});
    const action=await modal(replace?"Replace period analysis":"Add saved analysis",body=>{
      const search=input("search"),source=select([]),summary=el("p",null,"job-metadata");source.required=true;search.placeholder="Find an analysis by name";
      const available=analyses.filter(a=>!membership.has(a.id)),assigned=el("div");
      function filterAnalyses(){const previous=source.value,found=available.filter(a=>a.name.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase()));source.replaceChildren();const placeholder=el("option",found.length?"Choose a saved analysis…":"No matching analyses");placeholder.value="";source.append(placeholder);for(const a of found){const option=el("option",`${a.name}${a.archived?" (archived)":""}`);option.value=a.id;source.append(option);}source.value=found.some(a=>a.id===previous)?previous:"";describe();assigned.replaceChildren();for(const a of analyses.filter(a=>membership.has(a.id)&&a.name.toLocaleLowerCase().includes(search.value.trim().toLocaleLowerCase()))){const m=membership.get(a.id),row=el("p");row.append(el("span",`${a.name} — already in “${m.name}”${m.archived?" (archived)":""}. `),button("Open series",async()=>{body.closest("dialog").close();await open(m.series);}));assigned.append(row);}}
      function describe(){const a=available.find(a=>a.id===source.value);summary.textContent=a?`${a.owner?.name||""}${a.created?" · Saved "+new Date(a.created*1000).toLocaleDateString():""}`:`${available.length} saved analyses available. Each analysis can belong to only one series. Analyses already assigned are listed below.`;}
      search.oninput=filterAnalyses;source.onchange=describe;filterAnalyses();
      const start=input("date",replace?.start||""),end=input("date",replace?.end||"");start.required=end.required=true;if(replace)start.disabled=end.disabled=true;
      body.append(el("p","Step 1 of 3 — Choose an analysis and its period. Next, matches are calculated and saved; you then review uncertain matches and see the chart."),el("p",`Choose the centrally saved analysis to ${replace?"use as the replacement":"add to “"+current.name+"”"}. You do not need to load it in Results first.`),field("Search saved analyses",search),field("Saved analysis",source),summary,field("Period start",start),field("Period end",end),el("p","For example: 1 January–31 January for a January analysis. Dates are inclusive; all processed records remain included. Periods must not overlap."));
      body.append(assigned);
      const help=el("details");help.append(el("summary","Cannot find your analysis?"),el("p","A file or a job result is not yet a saved analysis. Go to Source, use Open CSV/XLSX and run clustering, or Load Session for a .icas file. Then click Save centrally. Come back here to choose it. If it already belongs to a series, open that series instead; add subsequent periods to the same series. Archived series retain their analyses and can be restored."),button("Go to Source to prepare or import",()=>prepareAnalysis(body)));body.append(help);
      return()=>{if(!source.value)throw Error("Choose an analysis");if(end.value<start.value)throw Error("End date precedes start date");if(current.periods.some(p=>p.id!==replace?.id&&start.value<=p.end&&end.value>=p.start))throw Error("These dates overlap an existing period. Choose a separate period, or use Replace for a rerun.");return{type:"addPeriod",analysis:source.value,start:start.value,end:end.value,replace:replace?.id||null};};
    },{confirmLabel:replace?"Review replacement":"Add and match"});if(action){
      const historical=current.periods.some(p=>p.start>action.start);
      const result=await command(action,base,sid);
      if(!replace&&!historical)await guidedTask(result.task,sid);
      else if(result.task)await guidedTask(result.task,sid,false,true);
    }
  }
  async function guidedTask(taskId,sid,showMatches=true,reviewImpact=false){
    let applied=false,analysis=null;
    await modal("Step 2 of 3 — Matching clusters",body=>{
      const status=el("p","Preparing the comparison…");status.setAttribute("role","status");
      body.append(el("p","Strong, unambiguous matches are saved automatically. Uncertain matches stay pending for your review. You can continue later from the task shown in this series."),status);
      const dialog=body.closest("dialog");
      (async()=>{try{
        while(dialog.isConnected){
          const task=await api(`/api/series/${sid}/previews/${taskId}`);
          if(!dialog.isConnected)return;
          if(current?.id!==sid)throw Error("The selected series changed. Return to the series to continue.");
          status.textContent=task.progress||task.state;
          if(task.state==="failed")throw Error(task.error||"Matching failed. Return to the series and retry.");
          if(task.state==="applied"){applied=true;analysis=task.action?.analysis;dialog.close();break;}
          if(task.state==="preview"){
            const memberships=await api("/api/series/memberships");
            const conflicting=memberships.some(m=>m.series!==sid&&task.proposed?.periods.some(p=>p.analysis===m.analysis));
            if(reviewImpact||conflicting){dialog.close();await preview(task);return;}
            status.textContent="Saving matches…";
            await api(`/api/series/${sid}/previews/${taskId}/apply`,post({commandId:`guided-${taskId}`}));
            applied=true;analysis=task.action?.analysis;dialog.close();break;
          }
          await new Promise(resolve=>setTimeout(resolve,750));
        }
      }catch(error){status.textContent=error.message;status.className="dashboard-error";dialog.querySelector('button[type="submit"]').textContent="Back to series";}})();
      return()=>false;
    },{confirmLabel:"Continue later",cancel:false});
    if(applied&&current?.id===sid){await refresh();while(busy)await new Promise(resolve=>setTimeout(resolve,50));message.textContent="Analysis saved and chart updated. Review any uncertain matches, or change a match at any time.";if(showMatches)await guideMatches(analysis);}
  }
  async function guideMatches(analysis){
    const sid=current.id;
    await modal("Step 3 of 3 — Review matches and view trends",body=>{
      let limit=50;
      const content=el("div");body.append(content);
      function draw(){
        if(current?.id!==sid){body.closest("dialog").close();return;}
        content.replaceChildren();const items=current.periods.filter(p=>!analysis||p.analysis===analysis).flatMap(p=>p.occurrences.map(o=>({p,o})));
        const pending=items.filter(({o})=>!o.problem).length;
        content.append(el("p",`${items.length-pending} clusters matched; ${pending} need review. Counts and the chart are already saved. Pending clusters stay unknown until you link them; you may return later.`));
        const ordered=[...items].sort((a,b)=>Number(!!a.o.problem)-Number(!!b.o.problem));
        for(const {p,o} of ordered.slice(0,limit)){
          const row=el("div",null,"series-review-row"),name=current.problems.find(v=>v.id===o.problem)?.name||"Pending — choose a match";
          row.append(el("strong",`${p.start} · Cluster ${o.cluster} — ${o.label}`),el("span",`${name} · ${o.count} records`),button(o.problem?"Change match":"Review match",async()=>{await link(p,o,true);draw();}));content.append(row);
        }
        if(ordered.length>limit)content.append(button("More clusters",()=>{limit+=50;draw();}));
      }
      draw();return()=>true;
    },{confirmLabel:"View updated chart",cancel:false});
    if(current?.id===sid)detail.querySelector(".series-chart")?.scrollIntoView({block:"start"});
  }
  async function link(period,occurrence,guided=false){
    const base=current.revision,sid=current.id;
    const action=await modal("Review cluster association",body=>{
      body.append(el("p",`${period.name} · ${period.start} – ${period.end}. Whole clusters contribute to at most one tracked problem. Select several clusters to combine a split manually.`));
      const clusters=select(period.occurrences.map(o=>[String(o.cluster),`Cluster ${o.cluster} — ${o.label} (${o.count})`]));clusters.multiple=true;clusters.size=Math.min(8,period.occurrences.length);for(const o of clusters.options)o.selected=Number(o.value)===occurrence.cluster;
      const target=select([["","Leave pending"],["__new","Create new tracked problem"],...current.problems.map(p=>[p.id,p.name])],occurrence.problem||occurrence.candidates[0]?.problem||"");const name=input("text",occurrence.label),revoke=input("checkbox");
      body.append(field("Contributing clusters",clusters),field("Tracked problem",target),field("Name for new problem",name),field("Revoke a conflicting confirmed zero",revoke));
      if(occurrence.candidates.length){const list=el("ul");for(const c of occurrence.candidates.slice(0,10)){const item=el("li",`${current.problems.find(p=>p.id===c.problem)?.name}: similarity ${c.score.toFixed(3)}, reference ${c.reference.toFixed(3)} (not a probability). Shared terms: ${(c.evidence||[]).join(", ")}`);const prior=current.periods.find(p=>p.id===c.period);if(prior)item.append(button(`Inspect ${prior.start} · Cluster ${c.cluster}`,()=>openReviewTarget(openAnalysis,prior.analysis,c.cluster)));list.append(item);}body.append(list);}
      return()=>{const ids=[...clusters.selectedOptions].map(o=>Number(o.value));if(!ids.length)throw Error("Select a cluster");return{type:"link",period:period.id,clusters:ids,problem:target.value&&target.value!=="__new"?target.value:null,newName:target.value==="__new"?name.value:null,revokeZero:revoke.checked};};
    },{confirmLabel:guided?"Confirm match":"Continue"});if(action){const result=await command(action,base,sid);if(guided)await guidedTask(result.task,sid,false);}
  }
  async function openCell(c,p){if(c.clusters.length===1)return openReviewTarget(openAnalysis,p.analysis,c.clusters[0]);const target=await modal("Contributing clusters",body=>{const chooser=select(c.clusters.map(id=>{const o=p.occurrences.find(o=>o.cluster===id);return[String(id),`Cluster ${id} — ${o?.label} (${o?.count} records)`];}));body.append(field("Open cluster",chooser));return()=>chooser.value;});if(target)await openReviewTarget(openAnalysis,p.analysis,target);}
  async function preview(task){
    const sid=current.id,base=current.revision;const data=await api(`/api/series/${sid}/previews/${task.id}`);
    if(!data.proposed)return noticeDialog(data.error||"Proposal is not ready.");
    if(data.expected!==base)return noticeDialog("This preview is stale. Refresh and retry the original operation.");
    async function resolveMembership(){
      const memberships=await api("/api/series/memberships");
      const conflicts=memberships.filter(m=>m.series!==sid&&data.proposed.periods.some(p=>p.analysis===m.analysis));
      if(!conflicts.length)return false;
      await modal("Analysis already in a series",body=>{
        body.append(el("p","This proposal cannot be saved because each analysis can belong to only one series. Open the existing series to continue there, or choose another analysis for this series."));
        for(const m of conflicts){const p=data.proposed.periods.find(p=>p.analysis===m.analysis);body.append(el("p",`${p.name} belongs to “${m.name}”${m.archived?" (archived — restore it to edit)":""}.`),button(`Open ${m.name}`,async()=>{body.closest("dialog").close();await open(m.series);}));}
        body.append(button("Choose another analysis",async()=>{body.closest("dialog").close();await addPeriod();}));
        return()=>true;
      },{confirmLabel:"Back to series",cancel:false});return true;
    }
    if(await resolveMembership())return;
    const accepted=await modal("Review impact before saving",body=>{
      body.append(el("p","Save to series accepts the complete proposal below. Cancel returns without saving. After saving, use Cluster associations → Review link to adjust individual matches. Existing manual links remain protected unless their source analysis is replaced."));
      const list=el("ul",null,"series-impact");for(const line of changes(current,data.proposed))list.append(el("li",line));if(!list.childElementCount)list.append(el("li","No association changes. Save the requested decision and its history."));body.append(list);
      const protectedCount=data.proposed.periods.flatMap(p=>p.occurrences).filter(o=>o.manual).length;body.append(el("p",`${protectedCount} protected manual decisions retained.`));
      return()=>{if(current.id!==sid||current.revision!==base)throw Error("Series changed while preview was open. Cancel and refresh.");return true;};
    },{confirmLabel:"Save to series"});if(accepted){try{await api(`/api/series/${sid}/previews/${task.id}/apply`,post({commandId:crypto.randomUUID()}));}catch(e){if(await resolveMembership())return;throw e;}message.textContent="Saved to series. Add another saved analysis to compare periods, or review cluster links below.";await refresh();}
  }
  function render(tasks){
    const revision=current.revision,sid=current.id,send=action=>command(action,revision,sid);
    detail.replaceChildren();const head=el("div",null,"section-head"),tools=el("div",null,"toolbar");head.append(el("h2",current.name));
    const edit=()=>!current.archived;
    if(edit())tools.append(button("Add saved analysis",()=>addPeriod()),button("Rename",async()=>{const name=await askDialog("Rename series","Names are shared and unique.",{value:current.name,label:"Series name"});if(name)await send({type:"rename",name});}),button("Recalculate",()=>send({type:"recalculate"})),button("Rebuild matching model",async()=>{if(await askDialog("Rebuild matching model","Fit a new reference from the earliest period and apply the operator’s current validation configuration? All consequences require a preview; manual links stay protected."))await send({type:"rebuildModel"});}));
    tools.append(button(current.archived?"Restore series":"Archive series",async()=>{if(await askDialog("Series archive",current.archived?"Restore this series for editing?":"Make this series read-only? Its history remains available."))await send({type:"archive",archived:!current.archived});}));if(current.periods.length&&!current.archived)tools.append(button("Review matches",()=>guideMatches()));head.append(tools);detail.append(head);
    tools.append(button("Delete series",async()=>{
      const name=current.name;
      const confirmed=await modal("Permanently delete series",body=>{
        body.append(el("p",`Permanently delete “${name}”? This removes all its periods, tracked problems, matching decisions, history, and pending proposals. This cannot be undone.`),el("p","Saved analyses and their review content are kept. They become available for another series, and this series name can be reused."));
        const check=input("checkbox");check.required=true;body.append(field("I understand that this series will be permanently deleted",check));
        return()=>{if(current?.id!==sid||current.revision!==revision)throw Error("Series changed. Cancel and review it before deleting.");return check.checked;};
      },{confirmLabel:"Permanently delete"});
      if(!confirmed)return;
      await api(`/api/series/${sid}/delete`,post({commandId:crypto.randomUUID(),expected:revision}));
      deletedSeries(sid);await refreshCatalog();
    }));
    if(current.archived)detail.append(el("p","Archived series — restore to change links or periods.","job-metadata"));
    if(current.model&&!current.model.automatic)detail.append(el("p","Automatic linking awaits historical-data validation. Candidates are available for manual confirmation; similarity scores are not probabilities.","job-metadata"));
    const taskList=el("div",null,"series-tasks");for(const t of tasks.filter(t=>t.state!=="applied").slice(0,8)){const row=el("div",null,"dashboard-row");row.append(el("span",`${t.state}: ${t.error||t.progress}`));if(["queued","running"].includes(t.state)&&t.action?.type==="addPeriod"&&edit())row.append(button("Continue setup",()=>guidedTask(t.id,sid,true,!!t.action.replace||current.periods.some(p=>p.start>t.action.start))));if(t.state==="preview"&&edit()){if(t.expected===current.revision)row.append(button(t.action?.type==="addPeriod"&&!t.action.replace?"Continue setup":"Review proposal",()=>t.action?.type==="addPeriod"&&!t.action.replace?guidedTask(t.id,sid,true,current.periods.some(p=>p.start>t.action.start)):preview(t)));else row.append(el("span","Stale — retry the original operation","job-metadata"));}if(edit()&&(t.state==="failed"||t.state==="preview"&&t.expected!==current.revision))row.append(button("Retry with current state",async()=>{await api(`/api/series/${sid}/tasks/${t.id}/retry`,post({commandId:crypto.randomUUID(),expected:revision}));await refresh();}));taskList.append(row);}detail.append(taskList);
    const periods=el("details");periods.dataset.key="periods";periods.append(el("summary",`Periods (${current.periods.length})`));for(const p of current.periods){const row=el("div",null,"dashboard-row");row.append(el("span",`${p.start} – ${p.end} · ${p.name} · ${p.total.toLocaleString()} processed records`));if(edit())row.append(button("Replace",()=>addPeriod(p)));periods.append(row);}detail.append(periods);
    if(current.periods.length===1)detail.append(el("p","Your first period is saved. Click Add saved analysis to add a second period and compare cluster counts.","series-next-step"));
    if(!current.periods.length){detail.append(el("p","Next: click Add saved analysis above, choose an analysis and its dates, then follow the matching wizard.","dashboard-empty"));return;}
    const chartTools=el("div",null,"toolbar"),metric=select([["count","Record count"],["percent","Percentage"]],percentage?"percent":"count");metric.onchange=()=>{percentage=metric.value==="percent";draw();};chartTools.append(field("Chart measure",metric));detail.append(chartTools);
    const legend=el("details");legend.dataset.key="legend";legend.append(el("summary","Choose chart lines (up to 12)"));const legendList=el("div",null,"series-legend");for(const r of rows){const check=input("checkbox");check.checked=selected.has(r.problem.id);check.onchange=()=>{if(check.checked&&selected.size>=12){check.checked=false;noticeDialog("Select up to 12 chart lines.");return;}check.checked?selected.add(r.problem.id):selected.delete(r.problem.id);draw();};legendList.append(field(r.problem.name,check));}legend.append(legendList);detail.append(legend);
    const chart=el("div"),chartNames=el("div",null,"series-legend");detail.append(chart,chartNames);function draw(){const chosen=rows.filter(r=>selected.has(r.problem.id));chart.replaceChildren(renderSeriesChart(current.periods,chosen,percentage,(cell,index)=>openCell(cell,current.periods[index]).catch(report)));chartNames.replaceChildren();chosen.forEach((r,i)=>chartNames.append(el("span",`${i+1}. ${r.problem.name}`,`series-key series-color-${i%6}`)));}draw();
    const wrap=el("div",null,"table-wrap series-table-wrap"),table=el("table"),thead=el("thead"),tr=el("tr");tr.append(el("th","Tracked problem"));for(const p of current.periods)tr.append(el("th",`${p.start} – ${p.end}`));thead.append(tr);table.append(thead);const tbody=el("tbody");table.append(tbody);let shown=0;const more=button("More tracked problems",()=>appendRows());
    function appendRows(){for(const r of rows.slice(shown,shown+100)){const tr=el("tr"),name=el("td");name.append(el("span",r.problem.name));if(edit())name.append(button("Rename",async()=>{const value=await askDialog("Rename tracked problem","Analysis labels remain unchanged.",{value:r.problem.name,label:"Problem name"});if(value)await send({type:"renameProblem",problem:r.problem.id,name:value});}));tr.append(name);for(const c of r.cells){const td=el("td"),p=current.periods.find(p=>p.id===c.period);const text=c.count==null?"No matched cluster":`${c.count.toLocaleString()} (${c.percent==null?"N/A":c.percent.toFixed(1)+"%"})`;
      if(c.clusters.length)td.append(button(text,()=>openCell(c,p)));else{td.append(el("span",text));if(c.zero)td.append(el("small",`Confirmed by ${c.zero.actor.name} · ${new Date(c.zero.time*1000).toLocaleString()}`));if(edit())td.append(button(c.zero?"Revoke zero":"Confirm zero",()=>send({type:"zero",period:p.id,problem:r.problem.id,confirmed:!c.zero})));}tr.append(td);}tbody.append(tr);}shown+=100;more.hidden=shown>=rows.length;}
    appendRows();wrap.append(table);detail.append(wrap,more);
    const review=el("details");review.dataset.key="review";const occurrences=current.periods.flatMap(p=>p.occurrences.map(o=>({p,o})));review.append(el("summary",`Cluster associations (${occurrences.filter(v=>!v.o.problem).length} pending)`));let limit=0;const reviewMore=button("More clusters",()=>appendReview());
    function appendReview(){for(const {p,o} of occurrences.slice(limit,limit+100)){const row=el("div",null,"series-review-row");const problem=current.problems.find(v=>v.id===o.problem);row.append(el("strong",`${p.start} · Cluster ${o.cluster} — ${o.label}`),el("span",`${o.count} records · ${problem?.name||"Pending"} · ${o.reason}`));const actions=el("div",null,"toolbar");actions.append(button("Open cluster",()=>openReviewTarget(openAnalysis,p.analysis,o.cluster)));if(edit()){actions.append(button("Review link",()=>link(p,o)));if(o.manual||o.replacement)actions.append(button("Reopen matching",()=>send({type:"reopen",period:p.id,cluster:o.cluster})));for(const c of o.candidates.slice(0,3))actions.append(button(`Reject ${current.problems.find(p=>p.id===c.problem)?.name||"candidate"}`,()=>send({type:"reject",period:p.id,cluster:o.cluster,problem:c.problem,reopen:false})));for(const rejected of o.rejected)actions.append(button(`Reconsider ${current.problems.find(p=>p.id===rejected)?.name||"candidate"}`,()=>send({type:"reject",period:p.id,cluster:o.cluster,problem:rejected,reopen:true})));}row.append(actions);review.insertBefore(row,reviewMore);}limit+=100;reviewMore.hidden=limit>=occurrences.length;}
    review.append(reviewMore);appendReview();detail.append(review);
    detail.append(button("Change history",async()=>{const history=await api(`/api/series/${current.id}/history`);await modal("Recent series changes",body=>{body.append(el("p","Most recent 50 changes. Full audit history is retained centrally."));for(const h of history){const d=el("details");d.append(el("summary",`${new Date(h.time*1000).toLocaleString()} · ${h.actor.name} · ${h.command.action.type}`));const ul=el("ul");for(const change of changes(h.before,h.after))ul.append(el("li",change));if(!ul.childElementCount)ul.append(el("li",JSON.stringify(h.command.action)));d.append(ul);body.append(d);}return()=>true;});}));
  }
  window.addEventListener("pane-change",e=>{active=e.detail==="series";if(active){refreshCatalog().catch(report);refresh().catch(report);}else clearTimeout(taskTimer);});
  window.addEventListener("series-refresh",e=>{if(e.detail?.resourceKind!=="series")return;if(active)refreshCatalog().catch(report);if(e.detail.change.deleted){deletedSeries(e.detail.seriesId);return;}if(current?.id===e.detail.seriesId){if(e.detail.change.revision>current.revision)message.textContent="Shared changes received.";refresh().catch(report);}});
  window.addEventListener("dashboard-refresh",()=>{if(active){refreshCatalog().catch(report);refresh().catch(report);}});
}
