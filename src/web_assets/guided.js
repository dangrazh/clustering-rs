// "Create new analysis": a clean start that guides the user from 1 Source to 4 Results.
import {state} from "./state.js";
import {clearProgress,syncSettings} from "./analysis.js";
import {askDialog,setStatus,showStep} from "./ui.js";

// Captured at import, before any analysis changes the settings.
const defaultSettings=structuredClone(state.settings);
const steps={
  source:["Step 1 of 4 — Source","Open the CSV or Excel export to analyse with Open CSV/XLSX. When the file has several worksheets, the first one is used; you can switch worksheets here."],
  mapping:["Step 2 of 4 — Mapping","Check that the incident number and short description are mapped, add optional fields, then choose Confirm Field Mapping."],
  analysis:["Step 3 of 4 — Analysis","Adjust the cluster settings if needed, then choose Start Analysis. Progress appears below; Results open when clustering finishes."],
  results:["Step 4 of 4 — Results","Review the clusters and themes. Choose Save centrally to share this analysis with your team; until then it stays a temporary result in your jobs."],
};
let guide=null;

function render(step){
  guide?.remove();guide=null;
  if(!state.guided||!steps[step])return;
  const [title,text]=steps[step],pane=document.getElementById(step);
  guide=document.createElement("div");guide.className="guided-step";guide.setAttribute("role","status");
  const heading=document.createElement("strong");heading.textContent=title;
  const body=document.createElement("p");body.textContent=text;
  guide.append(heading,body);
  if(step==="results"){
    const finish=document.createElement("button");finish.type="button";finish.textContent="Finish guide";
    finish.onclick=()=>{state.guided=false;render(step);};
    guide.append(finish);
  }
  pane.querySelector(".section-head")?.after(guide);
}

function clearWorkspace(){
  Object.assign(state,{
    source:null,mapping:null,analysis:null,jobId:null,analysisId:null,shared:null,selection:null,
    settings:structuredClone(defaultSettings),detailDrilldownRowIndices:null,detailDrilldownLabel:"",
    selectedPivotRowIndices:null,selectedPivotRowLabel:"",
  });
  for(const id of ["sourceStats","previewTable","worksheetBar","sourceColumnTypes","mappingForm","additionalText","mappingColumnTypes","mappingNotice"]){
    const element=document.getElementById(id);if(element)element.replaceChildren();
  }
  document.getElementById("worksheetBar")?.classList.add("hidden");
  clearProgress();document.getElementById("progressMessage").textContent="Waiting to start.";
  for(const step of ["mapping","analysis","results","pivot"])document.querySelector(`[data-step="${step}"]`).disabled=true;
  document.getElementById("sourceStatus").textContent="No source loaded";
  syncSettings();
}

export async function startNewAnalysis(){
  if(state.analysis&&!state.analysisId&&!await askDialog("Create new analysis","The current result is not saved centrally. It stays available under your jobs for its retention period. Start a new analysis?",{confirmLabel:"Start new analysis"}))return;
  clearWorkspace();
  state.guided=true;
  setStatus("New analysis: open a CSV or Excel file to begin.");
  showStep("source");
}

export function bindGuidedFlow(){
  const button=document.getElementById("newAnalysis");
  if(button)button.onclick=()=>startNewAnalysis();
  window.addEventListener("pane-change",e=>render(e.detail));
}
