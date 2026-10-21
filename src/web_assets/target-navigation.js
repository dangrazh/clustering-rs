import {state} from "./state.js";
import {renderResults,treeVisibleRows} from "./results.js";
import {effectiveWorkflow} from "./workflow.js";

export async function openReviewTarget(openAnalysis,aid,entity){
  if(await openAnalysis(aid)===false||state.analysisId!==aid)return;
  const [clusterId,themeId]=String(entity).split(":").map(Number);
  const cluster=state.analysis.clusters.find(c=>c.id===clusterId);
  const theme=themeId==null?null:cluster?.subgroups.find(t=>t.id===themeId);
  if(!cluster||themeId!=null&&!theme)throw Error("This review item is no longer available.");
  state.selection=theme?{type:"theme",cluster:clusterId,theme:themeId}:{type:"cluster",cluster:clusterId};
  state.treeLabelFilter="";
  state.expandedClusters.add(String(clusterId));
  const visible=treeVisibleRows();
  if(visible&&!(theme||cluster).incident_row_indices.some(row=>visible.has(row))){
    state.workflowStates=[];state.reviewerFilter={};state.detailColumnFilters=[];
    state.detailDrilldownRowIndices=null;state.detailDrilldownLabel="";
  }
  if(theme&&state.workflowStates.length&&!state.workflowStates.includes(effectiveWorkflow(state.review,entity).status))state.workflowStates=[];
  state.detailPage=1;renderResults();
}
