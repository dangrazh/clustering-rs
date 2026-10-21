// Missing observations remain gaps. The adjacent table is the accessible source of truth.
export function renderSeriesChart(periods,rows,percentage=false,onCell=null){
  const ns="http://www.w3.org/2000/svg",svg=document.createElementNS(ns,"svg");
  svg.setAttribute("viewBox","0 0 900 240");svg.setAttribute("role",onCell?"group":"img");
  svg.setAttribute("aria-label",`Cluster trends: ${percentage?"percentage of processed records":"record counts"}. Exact values and source links follow in the table.`);
  svg.classList.add("series-chart");
  const add=(tag,attrs,text)=>{const e=document.createElementNS(ns,tag);for(const [k,v]of Object.entries(attrs))e.setAttribute(k,v);if(text!=null)e.textContent=text;svg.append(e);return e;};
  if(!periods.length)return svg;
  const times=periods.map(p=>Date.parse(`${p.start}T00:00:00Z`));
  const min=Math.min(...times),span=Math.max(...times)-min||1;
  const value=c=>percentage?c.percent:c.count;
  const max=Math.max(1,...rows.flatMap(r=>r.cells.map(c=>value(c)||0)));
  const x=i=>50+(times[i]-min)/span*790,y=v=>200-v/max*170;
  add("path",{d:"M50 25V200H850",class:"series-axis"});
  add("text",{x:4,y:30},percentage?`${max.toFixed(1)}%`:String(max));add("text",{x:20,y:204},"0");
  add("text",{x:50,y:230},periods[0].start);if(periods.length>1)add("text",{x:850,y:230,"text-anchor":"end"},periods.at(-1).end);
  rows.forEach((r,index)=>{
    let d="",previous=false;
    r.cells.forEach((c,i)=>{
      const v=value(c);if(v==null){previous=false;return;}
      const gap=i>0&&Date.parse(`${periods[i-1].end}T00:00:00Z`)+86400000<times[i];
      d+=`${previous&&!gap?"L":"M"}${x(i)} ${y(v)} `;previous=true;
      const dot=add("circle",{cx:x(i),cy:y(v),r:4,class:`series-line series-color-${index%6}`});
      const title=document.createElementNS(ns,"title");title.textContent=`${r.problem.name}: ${v}${percentage?"%":""} (${periods[i].start} – ${periods[i].end})${c.zero?" — confirmed zero":""}`;dot.append(title);
      if(onCell&&c.clusters?.length){dot.setAttribute("role","button");dot.setAttribute("tabindex","0");dot.setAttribute("aria-label",`Open ${title.textContent}`);dot.style.cursor="pointer";dot.addEventListener("click",()=>onCell(c,i));dot.addEventListener("keydown",e=>{if(e.key==="Enter"||e.key===" "){e.preventDefault();onCell(c,i);}});}
    });
    add("path",{d,class:`series-line series-color-${index%6}`,fill:"none","stroke-dasharray":index>=6?"6 3":"none"});
  });return svg;
}
