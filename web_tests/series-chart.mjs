import test from "node:test";
import assert from "node:assert/strict";
import {renderBarChart,niceMax} from "../src/web_assets/series-chart.js";
class Node {
  constructor(tag){this.tag=tag;this.attributes={};this.children=[];this.listeners={};this.classList={add:name=>this.attributes.class=[this.attributes.class,name].filter(Boolean).join(" ")};}
  setAttribute(k,v){this.attributes[k]=v;}
  append(child){this.children.push(child);}
  addEventListener(type,listener){this.listeners[type]=listener;}
}
globalThis.document={createElementNS:(_,tag)=>new Node(tag)};
const groups=chart=>chart.children.filter(n=>n.tag==="g");
const bucket=(value,extra={})=>({label:"1 Jan",tooltip:`value ${value}`,value,clickable:false,...extra});
test("one bar per bucket with a value; zero draws no bar and unknown draws a dashed marker",()=>{
  const chart=renderBarChart([bucket(10),bucket(0),bucket(null),bucket(5)]);
  const marks=groups(chart).map(g=>g.children.find(n=>n.tag==="path")?.attributes.class??"none");
  assert.deepEqual(marks,["series-bar","none","series-unknown","series-bar"]);
  // Every column keeps a full-height hit target, even without a bar.
  assert.ok(groups(chart).every(g=>g.children.some(n=>n.tag==="rect"&&n.attributes.class==="series-hit")));
});
test("bars share the baseline and scale to a rounded maximum",()=>{
  assert.equal(niceMax(7),10);assert.equal(niceMax(230),250);assert.equal(niceMax(0),1);assert.equal(niceMax(1000),1000);
  const chart=renderBarChart([bucket(10),bucket(5)]);
  const ends=groups(chart).map(g=>g.children.find(n=>n.attributes.class==="series-bar").attributes.d.match(/V([\d.]+)Z$/)[1]);
  assert.equal(ends[0],ends[1]);
  const labels=chart.children.filter(n=>n.tag==="text").map(n=>n.textContent);
  assert.ok(labels.includes("10")&&labels.includes("5")&&labels.includes("0"));
});
test("only clickable buckets become keyboard-reachable buttons",()=>{
  const opened=[];
  const chart=renderBarChart([bucket(3,{clickable:true}),bucket(2)],{onBar:(b,i)=>opened.push(i)});
  const [first,second]=groups(chart);
  assert.equal(first.attributes.role,"button");assert.equal(first.attributes.tabindex,"0");assert.equal(second.attributes.role,undefined);
  first.listeners.keydown({key:"Enter",preventDefault(){}});first.listeners.click();
  assert.deepEqual(opened,[0,0]);
});
test("percentages label the axis with a percent sign",()=>{
  const labels=renderBarChart([bucket(42.5)],{percentage:true}).children.filter(n=>n.tag==="text").map(n=>n.textContent);
  assert.ok(labels.includes("50%"));
});
