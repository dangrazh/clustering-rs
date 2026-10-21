import {state} from "./state.js";
import {request} from "./api.js";
export function datePart(value,order="DMY"){
 const text=String(value??"").trim().split(/[T ]/)[0];let m=text.match(/^(\d{4})-(\d{1,2})-(\d{1,2})$/),y,month,d;
 if(m){[,y,month,d]=m.map(Number);}else{m=text.match(/^(\d{1,2})[./-](\d{1,2})[./-](\d{4})$/);if(!m)return null;y=Number(m[3]);month=Number(m[order==="MDY"?1:2]);d=Number(m[order==="MDY"?2:1]);}
 if(y<1||y>9999||month<1||month>12||d<1)return null;
 const leap=y%4===0&&(y%100!==0||y%400===0),days=[31,leap?29:28,31,30,31,30,31,31,30,31,30,31];if(d>days[month-1])return null;
 return `${String(y).padStart(4,"0")}-${String(month).padStart(2,"0")}-${String(d).padStart(2,"0")}`;
}
export function filterValue(value,columnType,mode="exact"){
 const text=String(value??"");if(columnType?.dataType!=="Date/Time"||mode==="exact")return text;
 const date=datePart(text,columnType.dateOrder);return date?(mode==="month"?date.slice(0,7):date):text;
}
export function renderColumnTypes(target,headers,columns,changed=()=>{}){
 target.replaceChildren();const intro=document.createElement("p");intro.textContent="Choose how each column is interpreted. Dates use day/month/year unless changed below. Source values remain unchanged; invalid dates remain available as exact values.";target.append(intro);
 const grid=document.createElement("div");grid.className="column-types-grid";target.append(grid);
 headers.forEach((name,index)=>{
  columns[index]||={dataType:"Text",dateOrder:"DMY"};const row=document.createElement("div"),label=document.createElement("label"),type=document.createElement("select"),order=document.createElement("select");row.className="column-type-row";
  label.textContent=name;for(const value of ["Text","Number","Date/Time"])type.add(new Option(value,value));type.value=columns[index].dataType;type.setAttribute("aria-label",`Data type: ${name}`);label.append(type);
  order.add(new Option("Day / month / year","DMY"));order.add(new Option("Month / day / year","MDY"));order.value=columns[index].dateOrder;order.setAttribute("aria-label",`Date order: ${name}`);order.hidden=type.value!=="Date/Time";
  type.onchange=()=>{columns[index].dataType=type.value;order.hidden=type.value!=="Date/Time";changed();};order.onchange=()=>{columns[index].dateOrder=order.value;changed();};row.append(label,order);grid.append(row);
 });
}
export function editColumnTypes(onSaved){
 const dialog=document.createElement("dialog");dialog.className="app-dialog series-dialog";dialog.setAttribute("aria-label","Column data types");const title=document.createElement("h2");title.textContent="Column data types";const body=document.createElement("div");body.className="series-dialog-body";const error=document.createElement("p");error.className="dashboard-error";error.setAttribute("role","alert");const actions=document.createElement("div");actions.className="dialog-actions";const cancel=document.createElement("button"),save=document.createElement("button");cancel.textContent="Cancel";save.textContent="Save column types";save.className="primary";actions.append(cancel,save);dialog.append(title,body,error,actions);document.body.append(dialog);
 const aid=state.analysisId,expected=state.shared?.columnTypesVersion||0,columns=structuredClone(state.analysis.column_types||[]);renderColumnTypes(body,state.analysis.source.headers,columns);
 save.disabled=!aid||!!state.shared?.analysis.archived;if(!aid)error.textContent="Save this analysis centrally before editing shared column types.";
 cancel.onclick=()=>dialog.close();save.onclick=async()=>{save.disabled=true;try{const response=await request(`/api/analyses/${aid}/commands`,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({commandId:crypto.randomUUID(),target:"",expected,action:{type:"columnTypes",columns}})});const data=await response.json();if(!response.ok)throw Error(data.error||"Could not save column types");if(state.analysisId===aid)onSaved(data.snapshot);dialog.close();}catch(e){error.textContent=e.message+" If another user changed the types, cancel and reopen this editor to review their changes.";save.disabled=false;}};
 dialog.onclose=()=>dialog.remove();dialog.showModal();
}
