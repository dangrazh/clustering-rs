// Sysadmin: upload static reference data and choose how it joins onto saved analyses.
import {request} from "./api.js";
import {askDialog} from "./ui.js";
import {json,post,node,empty,cell,dateCell,head,actionButton,replaceList,listPanel} from "./list-ui.js";

const typeLabels={Integer:"Whole number",Real:"Decimal number",Date:"Date",Text:"Text"};
// A modal built from a body; `read` returns the value to resolve with or throws a message.
function modal(title,build,{confirmLabel="Save",wide=false}={}){return new Promise(resolve=>{
  const dialog=node("dialog",null,`app-dialog series-dialog${wide?" static-dialog":""}`),heading=node("h2",title),form=node("form"),body=node("div",null,"series-dialog-body"),error=node("p",null,"dashboard-error"),actions=node("div",null,"dialog-actions");
  heading.id=`static-${crypto.randomUUID()}`;dialog.setAttribute("aria-labelledby",heading.id);error.setAttribute("role","alert");
  const cancel=node("button","Cancel");cancel.type="button";cancel.onclick=()=>dialog.close();const ok=node("button",confirmLabel,"primary");ok.type="submit";actions.append(cancel,ok);
  form.append(body,error,actions);dialog.append(heading,form);document.body.append(dialog);
  const read=build(body,dialog);let result=null;
  form.onsubmit=async e=>{e.preventDefault();ok.disabled=true;try{result=await read();dialog.close();}catch(x){error.textContent=x.message;}finally{ok.disabled=false;}};
  dialog.addEventListener("close",()=>{dialog.remove();resolve(result);},{once:true});dialog.showModal();
});}
function table(headers,rows){const t=node("table"),tr=node("tr");for(const h of headers)tr.append(node("th",h));const thead=node("thead");thead.append(tr);const tbody=node("tbody");for(const r of rows){const row=node("tr");for(const v of r){const td=node("td",v??"");td.title=v??"";row.append(td);}tbody.append(row);}t.append(thead,tbody);const wrap=node("div",null,"table-wrap static-preview");wrap.append(t);return wrap;}
async function upload(file){const r=await request(`/api/static-data/upload?filename=${encodeURIComponent(file.name)}`,{method:"POST",body:await file.arrayBuffer()});const v=await r.json();if(!r.ok)throw Error(v.error||r.statusText);return v;}

// Shows the inferred structure and creates (or replaces) the table.
async function reviewUpload(preview,replace){
  return modal(replace?`Replace data of “${replace.name}”`:"Create static data table",(body)=>{
    let current=preview;const name=node("input");name.value=replace?.name||preview.fileName.replace(/\.[^.]+$/,"");name.required=true;name.maxLength=200;name.disabled=!!replace;
    const nameField=node("label","Table name","field");nameField.append(name);
    const sheet=node("select");sheet.setAttribute("aria-label","Worksheet");for(const s of preview.worksheets)sheet.append(new Option(s,s));sheet.value=preview.worksheet||"";
    const sheetField=node("label","Worksheet","field");sheetField.append(sheet);sheetField.hidden=preview.worksheets.length<2;
    const details=node("div");
    function draw(){details.replaceChildren(node("p",`${current.rowCount.toLocaleString()} rows and ${current.columns.length} columns from ${current.fileName}${current.worksheet?` · ${current.worksheet}`:""}. Field names and types are inferred from the data.`,"job-metadata"),
      node("h3","Inferred structure"),table(["Column","Database field","Type"],current.columns.map(c=>[c.name,c.field,typeLabels[c.type]])),
      node("h3","First rows"),table(current.columns.map(c=>c.name),current.preview));}
    sheet.onchange=async()=>{details.replaceChildren(node("p","Reading worksheet…","job-metadata"));current=await json(`/api/static-data/upload/${preview.uploadId}/worksheet?sheet=${encodeURIComponent(sheet.value)}`,post({}));draw();};
    draw();
    if(replace)body.append(node("p","The rows are replaced. Join keys and added fields are kept where their columns still exist.","job-metadata"));
    body.append(nameField,sheetField,details);
    return()=>json("/api/static-data/create",post({uploadId:preview.uploadId,name:name.value,worksheet:current.worksheet||null,replace:replace?.id||null,expected:replace?.version??null}));
  },{confirmLabel:replace?"Replace data":"Create table",wide:true});
}

async function configure(t){
  return modal(`Join settings — ${t.name}`,(body)=>{
    const fieldName=f=>t.columns.find(c=>c.field===f)?.name||f;
    body.append(node("p","Rows join onto every saved analysis with a left outer join: all incidents stay, and matching values are added. Keys compare without regard to case or surrounding spaces. An analysis without all key columns is left unchanged.","job-metadata"));
    body.append(node("h3","Key fields"));
    const keys=node("div",null,"static-keys"),addKey=node("button","Add key field");addKey.type="button";
    function keyRow(key={field:t.columns[0].field,analysisColumn:""}){
      const row=node("div",null,"static-key-row"),field=node("select"),column=node("input"),remove=node("button","Remove");
      field.setAttribute("aria-label","Static data key field");for(const c of t.columns)field.append(new Option(c.name,c.field));field.value=key.field;
      column.setAttribute("aria-label","Analysis column");column.placeholder="Analysis column name";column.value=key.analysisColumn||fieldName(key.field);column.required=true;column.maxLength=300;
      field.onchange=()=>{if(!column.dataset.edited)column.value=fieldName(field.value);};column.oninput=()=>{column.dataset.edited="true";};
      remove.type="button";remove.onclick=()=>row.remove();
      row.append(field,node("span","matches","job-metadata"),column,remove);keys.append(row);
    }
    for(const k of t.config.keys)keyRow(k);if(!t.config.keys.length)keyRow();
    addKey.onclick=()=>keyRow();body.append(keys,addKey);
    body.append(node("h3","Fields to add to the analysis detail records"));
    const fields=node("div",null,"check-list static-fields");
    for(const c of t.columns){const label=node("label"),box=node("input");box.type="checkbox";box.value=c.field;box.checked=t.config.fields.includes(c.field);label.append(box,document.createTextNode(` ${c.name}`));fields.append(label);}
    body.append(fields);
    return async()=>{
      const config={keys:[...keys.querySelectorAll(".static-key-row")].map(r=>({field:r.querySelector("select").value,analysisColumn:r.querySelector("input").value.trim()})),fields:[...fields.querySelectorAll("input:checked")].map(b=>b.value)};
      if(config.fields.length&&!config.keys.length)throw Error("Add at least one key field, or clear the fields to add.");
      await json(`/api/static-data/${t.id}/config`,post({expected:t.version,config}));return true;
    };
  },{wide:true});
}

async function viewRows(t){
  return modal(`${t.name} — stored rows`,(body)=>{
    const rows=[],holder=node("div"),more=node("button","Show more rows");more.type="button";let offset=0;
    async function load(){const page=await json(`/api/static-data/${t.id}/rows?offset=${offset}`);rows.push(...page.rows);offset=page.nextOffset;holder.replaceChildren(table(t.columns.map(c=>c.name),rows));more.hidden=offset==null;}
    more.onclick=()=>load();body.append(node("p",`${t.rowCount.toLocaleString()} rows. Dates are stored as year-month-day.`,"job-metadata"),holder,more);load().catch(e=>holder.replaceChildren(node("p",e.message,"dashboard-error")));
    return()=>true;
  },{confirmLabel:"Close",wide:true});
}

export function staticDataSection(){
  const section=node("section",null,"static-data"),{panel,controls,error,list}=listPanel("static","Static data");
  section.append(panel);
  panel.insertBefore(node("p","Upload reference tables from Excel or CSV and join them onto saved analyses by key fields. Joined fields appear as extra columns in Results, filters, pivot and exports of every saved analysis that has the key columns. Stored analyses are not changed.","job-metadata static-intro"),controls);
  const file=node("input");file.type="file";file.accept=".xlsx,.xlsm,.xls,.csv";file.setAttribute("aria-label","Static data file");
  const pick=node("label","Upload Excel or CSV","button primary");pick.append(file);
  let replacing=null,tables=[];
  file.onchange=async()=>{
    const chosen=file.files[0],target=replacing;replacing=null;file.value="";if(!chosen)return;
    error.textContent=`Reading ${chosen.name}…`;
    try{const preview=await upload(chosen);error.textContent="";if(await reviewUpload(preview,target))await refresh();}catch(e){error.textContent=e.message;}
  };
  controls.append(pick);
  function render(){
    replaceList(list,l=>{
      if(!tables.length)return empty(l,"No static data yet. Upload an Excel or CSV file to create a table.");
      l.append(head("Table","Rows","Join keys","Added fields","Updated",""));
      for(const t of tables){
        const row=node("div",null,"dashboard-row"),actions=node("span",null,"dashboard-actions");
        const keyText=t.config.keys.map(k=>`${t.columns.find(c=>c.field===k.field)?.name||k.field} = ${k.analysisColumn}`).join(", ")||"Not set";
        const fieldText=t.config.fields.map(f=>t.columns.find(c=>c.field===f)?.name||f).join(", ")||"None";
        actions.append(
          actionButton("Join settings",async()=>{if(await configure(t))await refresh();}),
          actionButton("View rows",()=>viewRows(t)),
          actionButton("Replace data",()=>{replacing=t;file.click();}),
          Object.assign(actionButton("Delete",async()=>{if(!await askDialog("Delete static data table",`Delete “${t.name}” and its ${t.rowCount.toLocaleString()} rows? Its fields disappear from all analyses.`,{confirmLabel:"Delete"}))return;await json(`/api/static-data/${t.id}/delete`,post({}));await refresh();}),{className:"dashboard-action danger"}),
        );
        const name=cell(t.name,"dashboard-meta static-name");name.title=`${t.name} — ${t.source} — database table ${t.sqlName}`;
        row.append(name,cell(t.rowCount.toLocaleString(),"dashboard-meta dashboard-number"),cell(keyText),cell(fieldText),dateCell(new Date(t.created*1000)),actions);
        l.append(row);
      }
    });
  }
  async function refresh(){try{tables=await json("/api/static-data");error.textContent="";render();}catch(e){error.textContent=e.message;}}
  return {section,refresh};
}
