// Dense, table-like lists shared by the Dashboard and the Analyses and Jobs pane.
import {request} from "./api.js";
import {noticeDialog} from "./ui.js";
export const post=value=>({method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify(value)});
export async function json(url,options){const r=await request(url,options),v=await r.json();if(!r.ok)throw Error(v.error||r.statusText);return v;}
export const node=(tag,text,cls)=>{const e=document.createElement(tag);if(text!=null)e.textContent=text;if(cls)e.className=cls;return e;};
export const button=(text,action)=>{const b=node("button",text);b.type="button";b.onclick=()=>Promise.resolve().then(action).catch(e=>noticeDialog(e.message));return b;};
export const empty=(list,text)=>list.append(node("p",text,"dashboard-empty"));
// Cells truncate to keep rows on one line, so the full value stays available as a tooltip.
export const cell=(text,cls="dashboard-meta",title=text)=>{const e=node("span",text,cls);e.title=title;return e;};
export const shortDate=date=>date.toLocaleString([],{dateStyle:"short",timeStyle:"short"});
export const dateCell=date=>cell(shortDate(date),"dashboard-meta",date.toLocaleString());
export const count=value=>cell(value.toLocaleString(),"dashboard-meta dashboard-number");
export const head=(...labels)=>{const e=node("div",null,"dashboard-head");e.append(...labels.map(label=>node("span",label)));return e;};
export const openButton=(text,key,action)=>{const b=button(text,action);b.className="dashboard-open";b.title=text;b.dataset.key=key;return b;};
export const moreButton=(text,action)=>{const b=button(text,action);b.className="dashboard-more";return b;};
export const actionButton=(text,action)=>{const b=button(text,action);b.className="dashboard-action";return b;};
// A panel with a heading, a controls area, an error line and a scrolling list.
export function listPanel(key,title,idPrefix="list"){
  const panel=node("section",null,"dashboard-panel"),heading=node("h3",title),controls=node("div",null,"dashboard-controls"),error=node("p",null,"dashboard-error"),list=node("div",null,"dashboard-list");
  list.dataset.list=key;heading.id=`${idPrefix}-${key}-title`;panel.setAttribute("aria-labelledby",heading.id);error.setAttribute("role","status");panel.append(heading,controls,error,list);
  return {panel,controls,error,list};
}
// Rebuilds a list in place while keeping its scroll position and keyboard focus.
export function replaceList(list,build){const scroll=list.scrollTop;const focus=document.activeElement?.dataset?.key;list.replaceChildren();build(list);list.scrollTop=scroll;if(focus){const match=[...list.querySelectorAll("[data-key]")].find(e=>e.dataset.key===focus);match?.focus({preventScroll:true});}}
export function searchInput(label,value,onInput){const input=node("input");input.type="search";input.placeholder="Search by name";input.setAttribute("aria-label",label);input.value=value;input.oninput=()=>onInput(input.value.slice(0,200));return input;}
export function checkbox(text,checked,onChange){const label=node("label"),input=node("input");input.type="checkbox";input.checked=checked;input.onchange=()=>onChange(input.checked);label.append(input,document.createTextNode(` ${text}`));return label;}
export function select(label,options,value,onChange){const s=node("select");s.setAttribute("aria-label",label);for(const [v,text] of options){const o=node("option",text);o.value=v;s.append(o);}s.value=value;s.onchange=()=>onChange(s.value);return s;}
