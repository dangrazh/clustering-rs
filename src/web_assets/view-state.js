// Keep durable configuration separate from transient DOM and request state.
export function captureView(state) {
  return structuredClone({
    version: 1, selection: state.selection, expandedClusters: [...state.expandedClusters],
    detailColumnFilters: state.detailColumnFilters, detailSort: state.detailSort,
    pivotRows: state.pivotRows, pivotColumns: state.pivotColumns,
    detailDrilldownRowIndices: state.detailDrilldownRowIndices,
    detailDrilldownLabel: state.detailDrilldownLabel, workflowStates: state.workflowStates,
  });
}
export function restoreView(state, view) {
  if (!view) return;
  state.selection = view.selection || { type: "all" };
  state.expandedClusters = new Set(view.expandedClusters || []);
  for (const key of ["detailColumnFilters", "pivotRows", "pivotColumns", "workflowStates"]) state[key] = structuredClone(view[key] || []);
  state.detailSort = view.detailSort || null;
  state.detailDrilldownRowIndices = view.detailDrilldownRowIndices || null;
  state.detailDrilldownLabel = view.detailDrilldownLabel || "";
}
