import assert from "node:assert/strict";
import test from "node:test";
import { effectiveWorkflow, workflowRows, stateMatches } from "../src/web_assets/workflow.js";
import { captureView, restoreView } from "../src/web_assets/view-state.js";
import { treeVisibleRows } from "../src/web_assets/results.js";
import { state } from "../src/web_assets/state.js";
const run = {
  source: { rows: [["Open"], ["Closed"], ["Open"], ["Open"]] },
  processed_incidents: [0,1,2,3].map(source_row_index => ({ source_row_index })),
  clusters: [{ id: 1, incident_row_indices: [0,1,2], subgroups: [{ id:1, incident_row_indices:[0,1] }, { id:2, incident_row_indices:[2] }] }],
};
function review() { return { annotations: { revision:1, entries: { "1": { workflow: {status:"Reviewed"} }, "1:1": {workflow:null}, "1:2": {workflow:{status:"NoOpportunity"}} } } }; }
test("themes inherit, overrides stay independent, memberships form a union", () => {
  const data=review();
  assert.equal(effectiveWorkflow(data,"1:1").status,"Reviewed");
  assert.deepEqual([...workflowRows(run,data,["Reviewed"])],[0,1,2]);
  assert.deepEqual([...workflowRows(run,data,["NoOpportunity"])],[2]);
  assert.equal(stateMatches(data,"1",["NoOpportunity"]),false); // visible parent is context
  assert.equal(workflowRows(run,data,[]),null); // unrestricted includes unclustered
  const changed=structuredClone(data);changed.annotations.entries["1"].workflow.status="Review";
  assert.equal(effectiveWorkflow(changed,"1:1").status,"Review");
  assert.equal(effectiveWorkflow(changed,"1:2").status,"NoOpportunity");
  assert.deepEqual([...workflowRows(run,changed,["Reviewed"])],[]);
});
test("workflow filters intersect detail filters and pivot drilldown", () => {
  state.analysis=run;state.review=review();state.workflowStates=["Reviewed"];
  state.detailColumnFilters=[{selected:["Open"],query:"",searchDeselected:false}];state.detailDrilldownRowIndices=[1,2,3];
  assert.deepEqual([...treeVisibleRows()],[2]);
  state.workflowStates=["Review"];assert.deepEqual([...treeVisibleRows()],[]);
  state.workflowStates=[];assert.deepEqual([...treeVisibleRows()],[2,3]);
});
test("view state round trip restores independent copies and excludes identity", () => {
  const initial={selection:{type:"theme",cluster:1,theme:2},expandedClusters:new Set(["1"]),detailColumnFilters:[{selected:["Open"],query:"",searchDeselected:false}],detailSort:{column:0,direction:"desc"},pivotRows:[0],pivotColumns:[1],detailDrilldownRowIndices:[2],detailDrilldownLabel:"Open",workflowStates:["NoOpportunity"],actor:{name:"Local only"}};
  const wire=captureView(initial),restored={};restoreView(restored,JSON.parse(JSON.stringify(wire)));
  assert.deepEqual(captureView(restored),wire);assert.equal(wire.actor,undefined);
  initial.detailColumnFilters[0].selected.push("Closed");assert.deepEqual(restored.detailColumnFilters[0].selected,["Open"]);
});
