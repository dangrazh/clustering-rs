// One small bar chart per tracked problem. The table below the charts is the accessible source of truth.
// Each bucket: {label, tooltip, value (null = unknown), clickable}.
const W=360,H=150,LEFT=40,RIGHT=8,TOP=12,BOTTOM=22,PLOT=H-TOP-BOTTOM;

// Round the axis maximum up to 1, 2 or 5 × 10^n so gridline labels stay readable.
export function niceMax(value){
  if(!(value>0))return 1;const power=10**Math.floor(Math.log10(value)),step=[1,2,2.5,5,10].find(s=>s*power>=value);
  return step*power;
}
// Bar with a 4px rounded data end, anchored square on the baseline.
function barPath(x,y,width,height){
  const r=Math.min(4,width/2,height);
  return `M${x} ${y+height}V${y+r}Q${x} ${y} ${x+r} ${y}H${x+width-r}Q${x+width} ${y} ${x+width} ${y+r}V${y+height}Z`;
}
export function renderBarChart(buckets,{percentage=false,label="",onBar=null,onHover=null}={}){
  const ns="http://www.w3.org/2000/svg",svg=document.createElementNS(ns,"svg");
  svg.setAttribute("viewBox",`0 0 ${W} ${H}`);svg.setAttribute("role",onBar?"group":"img");svg.setAttribute("aria-label",label);
  svg.classList.add("series-bars");
  const add=(parent,tag,attrs,text)=>{const e=document.createElementNS(ns,tag);for(const [k,v]of Object.entries(attrs))e.setAttribute(k,v);if(text!=null)e.textContent=text;parent.append(e);return e;};
  const max=niceMax(Math.max(0,...buckets.map(b=>b.value??0))),y=v=>TOP+PLOT-v/max*PLOT,format=v=>percentage?`${Number(v.toFixed(1))}%`:v.toLocaleString();
  for(const v of [0,max/2,max]){add(svg,"path",{d:`M${LEFT} ${y(v)}H${W-RIGHT}`,class:v?"series-grid":"series-axis"});add(svg,"text",{x:LEFT-6,y:y(v)+4,"text-anchor":"end"},format(v));}
  if(!buckets.length)return svg;
  const band=(W-LEFT-RIGHT)/buckets.length,width=Math.max(1,Math.min(48,band-2));
  buckets.forEach((b,i)=>{
    const x=LEFT+i*band,g=add(svg,"g",{class:"series-bar-group"});
    if(b.value==null)add(g,"path",{d:`M${x+band/2-width/2} ${y(0)-1}H${x+band/2+width/2}`,class:"series-unknown"});
    else if(b.value>0)add(g,"path",{d:barPath(x+band/2-width/2,y(b.value),width,y(0)-y(b.value)),class:"series-bar"});
    // The hit target spans the whole column, so thin weekly bars stay easy to point at.
    add(g,"rect",{x,y:TOP,width:band,height:PLOT,class:"series-hit"});
    if(onHover){g.addEventListener("pointerenter",()=>onHover(b,(x+band/2)/W));g.addEventListener("pointerleave",()=>onHover(null));}
    if(onBar&&b.clickable){g.setAttribute("role","button");g.setAttribute("tabindex","0");g.setAttribute("aria-label",`Open ${b.tooltip}`);g.classList.add("clickable");g.addEventListener("click",()=>onBar(b,i));g.addEventListener("keydown",e=>{if(e.key==="Enter"||e.key===" "){e.preventDefault();onBar(b,i);}});}
  });
  // Label the first and last buckets, plus the middle one when there is room.
  const ticks=[...new Set([0,buckets.length>4?Math.floor((buckets.length-1)/2):null,buckets.length-1].filter(i=>i!=null))];
  for(const i of ticks){const anchor=buckets.length===1?"middle":i===0?"start":i===buckets.length-1?"end":"middle";const x=anchor==="start"?LEFT+i*band:anchor==="end"?LEFT+(i+1)*band:LEFT+(i+.5)*band;add(svg,"text",{x,y:H-6,"text-anchor":anchor},buckets[i].label);}
  return svg;
}
