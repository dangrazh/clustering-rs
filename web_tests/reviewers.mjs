import assert from 'node:assert/strict';
import test from 'node:test';
import {matchesReviewer,reviewerRows} from '../src/web_assets/reviewers.js';
import {captureView,restoreView} from '../src/web_assets/view-state.js';
test('reviewer categories use confirmed identities and union selected choices',()=>{
 const f={selected:true,users:['bob'],me:true,unassigned:true};
 assert.equal(matchesReviewer({userId:'bob'},f,'alice'),true);
 assert.equal(matchesReviewer({userId:'alice'},f,'alice'),true);
 assert.equal(matchesReviewer({},f,'alice'),true);
 assert.equal(matchesReviewer({userId:'other'},f,'alice'),false);
 assert.equal(matchesReviewer({userId:'alice',unconfirmed:true},f,'alice'),false);
 assert.equal(matchesReviewer({unconfirmed:true},{selected:true,unconfirmed:true},'alice'),true);
 assert.equal(matchesReviewer({},{}),true);
 assert.equal(matchesReviewer({},{selected:true}),false);
});
test('reviewer membership deduplicates incident rows and follows reassignment',()=>{
 const s={user:{id:'alice'},reviewerFilter:{selected:true,me:true},analysis:{clusters:[{id:1,incident_row_indices:[0,1,1]},{id:2,incident_row_indices:[1,2]}]},shared:{reviewers:{1:{value:{userId:'alice'}},2:{value:{userId:'bob'}}}}};
 assert.deepEqual([...reviewerRows(s)],[0,1]);s.shared.reviewers[1].value.userId='bob';assert.equal(reviewerRows(s).size,0);
 s.reviewerFilter={};assert.equal(reviewerRows(s),null);
});
test('reviewer filters survive personal view round trips including empty selected state',()=>{
 const s={expandedClusters:new Set(),reviewerFilter:{selected:true,users:[]}};const view=captureView(s),restored={};restoreView(restored,view);assert.deepEqual(restored.reviewerFilter,s.reviewerFilter);restored.reviewerFilter.users.push('bob');assert.deepEqual(s.reviewerFilter.users,[]);restoreView(restored,{version:1});assert.deepEqual(restored.reviewerFilter,{});
});
