// Run against a test server after generating target/smoke.icas.
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
const base=process.env.CLUSTERING_TEST_URL||'http://127.0.0.1:18080';
async function request(path,body,raw=false){const response=await fetch(base+path,body===undefined?{}:{method:'POST',headers:{'Content-Type':raw?'application/octet-stream':'application/json'},body:raw?body:JSON.stringify(body)});const type=response.headers.get('content-type');return{status:response.status,data:type?.includes('json')?await response.json():Buffer.from(await response.arrayBuffer())};}
const fixture=await fs.readFile('target/smoke.icas');
const restored=await request('/api/sessions',fixture,true);assert.equal(restored.status,200);const id=restored.data.jobId,prefix=`/api/jobs/${id}`;
let current=(await request(prefix+'/review')).data;
const actor={name:'API Reviewer',email:'api@example.org'};
const mutation=action=>({revision:current.review.annotations.revision,target:'1',actor,action});
let response=await request(prefix+'/review/mutate',mutation({type:'transition',status:'Solved',assignments:{}}));assert.equal(response.status,400);assert.deepEqual((await request(prefix+'/review')).data,current);
const stale=mutation({type:'undo'});response=await request(prefix+'/review/mutate',stale);assert.equal(response.status,200);current=response.data;
assert.equal((await request(prefix+'/review/mutate',stale)).status,409);
const bytes=(await request(prefix+'/review/save',{})).data;
const prepared=(await request(prefix+'/review/prepare',bytes,true)).data;
response=await request(prefix+'/review/mutate',mutation({type:'addComment',text:'Keep when import fails'}));assert.equal(response.status,200);current=response.data;
assert.equal((await request(prefix+'/review/commit',prepared)).status,409);assert.deepEqual((await request(prefix+'/review')).data,current);
const corrupt=Buffer.from(bytes);corrupt[20]^=0xff;assert.equal((await request(prefix+'/review/prepare',corrupt,true)).status,400);assert.deepEqual((await request(prefix+'/review')).data,current);
const valid=(await request(prefix+'/review/prepare',bytes,true)).data;assert.equal((await request(prefix+'/review/commit',{...valid,token:'wrong'})).status,400);
response=await request(prefix+'/review/commit',valid);assert.equal(response.status,200);assert.equal(response.data.review.annotations.revision,current.review.annotations.revision+1);
assert.equal((await request('/api/sessions',Buffer.from('{"version":2}'),true)).status,400);
const source=await request('/api/import?filename=workflow-api.csv',Buffer.from('Number,Short Description\nINC1,Network connectivity failure\nINC2,Network connectivity failure\n'),true);
assert.equal(source.status,200);
const started=await request('/api/analyze',{sourceId:source.data.sourceId,mapping:{incident_number:0,short_description:1,additional_text:[],assignment_group:null,service:null,category:null,configuration_item:null,date:null},settings:{minimum_cluster_size:2,similarity_threshold_percent:42,subgroup_similarity_threshold_percent:58,label_terms:{boosted:[],suppressed:[],excluded:[]}}});
assert.equal(started.status,202);
let fresh;
for(let attempt=0;attempt<100;attempt++){fresh=await request(`/api/jobs/${started.data.jobId}/review`);if(fresh.status===200)break;await new Promise(resolve=>setTimeout(resolve,50));}
assert.equal(fresh.status,200);assert.ok(Object.values(fresh.data.review.annotations.entries).some(entry=>entry.workflow?.status==='Review'));
assert.equal((await request(`/api/jobs/${started.data.jobId}/review/prepare`,bytes,true)).status,400);
console.log('API invalid mutations, stale revisions, atomic failed imports, confirmation tokens and binary errors passed.');
