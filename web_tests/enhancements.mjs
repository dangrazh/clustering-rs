import test from "node:test";
import assert from "node:assert/strict";
import {labelMatches,clearResultsFilters,captureView,restoreView} from "../src/web_assets/view-state.js";
import {reviewerRows,assignment} from "../src/web_assets/reviewers.js";
test("label search is a literal case-insensitive substring with star wildcards",()=>{
 assert.ok(labelMatches("Payment gateway timeout","PAY*TIME"));assert.ok(labelMatches("a[b].c","[b]."));assert.ok(!labelMatches("abc","a.c"));assert.ok(!labelMatches("timeout payment","pay*time"));assert.ok(labelMatches("anything","*"));
});
test("clear all filters preserves layout and sorting, and persists empty filter state",()=>{
 const s={expandedClusters:new Set(["1"]),treeLabelFilter:"pay*",workflowStates:["Review"],reviewerFilter:{selected:true},detailColumnFilters:[{selected:["x"]}],detailDrilldownRowIndices:[1],selection:{type:"cluster",cluster:1},detailPage:5,detailSort:{column:1},pivotRows:[2],pivotColumns:[3],detailPageSize:100};clearResultsFilters(s);assert.equal(s.selection.type,"all");assert.equal(s.detailPage,1);assert.equal(s.treeLabelFilter,"");assert.deepEqual(s.detailSort,{column:1});assert.deepEqual(s.pivotRows,[2]);const restored={};restoreView(restored,captureView(s));assert.deepEqual(restored.workflowStates,[]);assert.deepEqual(restored.detailColumnFilters,[]);assert.equal(restored.treeLabelFilter,"");
});
test("effective theme reviewer supports override, explicit unassigned, inheritance and remaining parent rows",()=>{
 const s={user:{id:"alice"},reviewerFilter:{selected:true,me:true},analysis:{clusters:[{id:1,incident_row_indices:[0,1,2],subgroups:[{id:1,incident_row_indices:[0]},{id:2,incident_row_indices:[1]}]}]},shared:{reviewers:{1:{value:{userId:"alice"}},"1:1":{value:{userId:"bob"}},"1:2":{value:{}}}}};
 assert.deepEqual([...reviewerRows(s)],[2]);assert.equal(assignment(s,"1:1").userId,"bob");s.shared.reviewers["1:1"].value=null;assert.deepEqual([...reviewerRows(s)],[0,2]);s.shared.reviewers[1].value={};s.shared.reviewers["1:2"].value={userId:"alice"};assert.deepEqual([...reviewerRows(s)],[1]);
});
