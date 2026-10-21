// Keep durable configuration separate from transient DOM and request state.
export function captureView(state) {
  return structuredClone({
    version: 4, dateFilterModes:state.dateFilterModes||[], treeLabelFilter: state.treeLabelFilter || "", reviewerFilter: state.reviewerFilter || {}, selection: state.selection, expandedClusters: [...state.expandedClusters],
    detailColumnFilters: state.detailColumnFilters, detailSort: state.detailSort,
    pivotRows: state.pivotRows, pivotColumns: state.pivotColumns,
    detailDrilldownRowIndices: state.detailDrilldownRowIndices,
    detailDrilldownLabel: state.detailDrilldownLabel, workflowStates: state.workflowStates,
  });
}
export function restoreView(state, view) {
  state.dateFilterModes=structuredClone(view?.dateFilterModes||[]);
  state.treeLabelFilter = view?.treeLabelFilter || "";
  state.reviewerFilter = structuredClone(view?.reviewerFilter || {});
  if (!view) return;
  state.selection = view.selection || { type: "all" };
  state.expandedClusters = new Set(view.expandedClusters || []);
  for (const key of ["detailColumnFilters", "pivotRows", "pivotColumns", "workflowStates"]) state[key] = structuredClone(view[key] || []);
  state.detailSort = view.detailSort || null;
  state.detailDrilldownRowIndices = view.detailDrilldownRowIndices || null;
  state.detailDrilldownLabel = view.detailDrilldownLabel || "";
}

export function labelMatches(label, query) {
  const parts=String(query||"").trim().toLocaleLowerCase().split("*");
  const text=String(label||"").toLocaleLowerCase();let offset=0;
  for(const part of parts){const index=text.indexOf(part,offset);if(index<0)return false;offset=index+part.length;}
  return true;
}
export function clearResultsFilters(s) {
  s.dateFilterModes=[];s.workflowStates=[];s.reviewerFilter={};s.treeLabelFilter="";s.detailColumnFilters=[];
  s.detailOpenFilterColumn=null;s.detailDrilldownRowIndices=null;s.detailDrilldownLabel="";
  s.selection={type:"all"};s.detailPage=1;s.selectedPivotRowIndices=null;s.selectedPivotRowLabel="";
}
