import test from "node:test";
import assert from "node:assert/strict";
import {renderSeriesChart} from "../src/web_assets/series-chart.js";
class Node {
  constructor(tag){this.tag=tag;this.attributes={};this.children=[];this.classList={add:name=>this.attributes.class=name};}
  setAttribute(k,v){this.attributes[k]=v;}
  append(child){this.children.push(child);}
}
globalThis.document={createElementNS:(_,tag)=>new Node(tag)};
const periods=[{start:"2026-01-01",end:"2026-01-31"},{start:"2026-02-01",end:"2026-02-28"},{start:"2026-03-01",end:"2026-03-31"}];
const row=counts=>({problem:{name:"Recurring problem"},cells:counts.map(count=>({count,percent:count,zero:count===0?{}:null}))});
test("missing observations break a line while confirmed zero is plotted",()=>{
  const chart=renderSeriesChart(periods,[row([10,null,0])]);
  const line=chart.children.find(n=>n.tag==="path"&&n.attributes.class?.includes("series-line"));
  assert.equal((line.attributes.d.match(/M/g)||[]).length,2);assert.ok(!line.attributes.d.includes("L"));
  const dots=chart.children.filter(n=>n.tag==="circle");assert.equal(dots.length,2);assert.equal(dots[1].attributes.cy,200);
  assert.match(dots[1].children[0].textContent,/confirmed zero/);
});
test("contiguous observations connect; uncovered calendar gaps do not",()=>{
  const contiguous=renderSeriesChart(periods,[row([3,2,1])]);assert.match(contiguous.children.find(n=>n.attributes.class?.includes("series-line")&&n.tag==="path").attributes.d,/L/);
  const gap=renderSeriesChart([periods[0],periods[2]],[row([3,1])]);assert.ok(!gap.children.find(n=>n.attributes.class?.includes("series-line")&&n.tag==="path").attributes.d.includes("L"));
});
test("undefined percentages do not become zeros",()=>{
  const chart=renderSeriesChart([periods[0]],[{problem:{name:"Empty analysis"},cells:[{count:0,percent:null,zero:{}}]}],true);
  assert.equal(chart.children.filter(n=>n.tag==="circle").length,0);
});
