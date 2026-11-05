// Measures user-visible loading/filtering/saving and Chromium JS heap on a release test server.
import {chromium} from '../target/browser-tools/node_modules/playwright/index.mjs';
import fs from 'node:fs/promises';
const browser=await chromium.launch({channel:'msedge',headless:true});
const results=[];
try {
  for (const count of [100000,150000,200000]) {
    const page=await browser.newPage({viewport:{width:1500,height:1000},acceptDownloads:true});
    const errors=[];page.on('pageerror',error=>errors.push(error.message));
    await page.goto(process.env.CLUSTERING_TEST_URL||'http://127.0.0.1:18081');
    const cdp=await page.context().newCDPSession(page);await cdp.send('Performance.enable');
    let peakHeap=0,maxHeartbeatGap=0;
    await page.evaluate(()=>{window.lastBeat=performance.now();window.maxBeatGap=0;setInterval(()=>{const now=performance.now();window.maxBeatGap=Math.max(window.maxBeatGap,now-window.lastBeat);window.lastBeat=now;},50);});
    let running=true;
    const sampling=(async()=>{while(running){const metrics=(await cdp.send('Performance.getMetrics')).metrics;peakHeap=Math.max(peakHeap,metrics.find(m=>m.name==='JSHeapUsedSize').value);await new Promise(resolve=>setTimeout(resolve,50));}})();
    let start=performance.now();await page.locator('#sessionInput').setInputFiles(`target/benchmark-${count}.icas`);await page.locator('#clusterList .cluster-label').first().waitFor({timeout:60000});
    const loadMs=Math.round(performance.now()-start);
    await page.locator('.workflow-filters summary').click();start=performance.now();await page.locator('#workflowFilters input[value=Reviewed]').check();await page.locator('#clusterList .cluster-label').first().waitFor();const filterMs=Math.round(performance.now()-start);
    start=performance.now();const downloaded=page.waitForEvent('download',{timeout:60000});await page.locator('#downloadSession').click();const download=await downloaded;await download.path();const saveMs=Math.round(performance.now()-start);
    maxHeartbeatGap=Math.round(await page.evaluate(()=>window.maxBeatGap));running=false;await sampling;
    const result={records:count,loadMs,filterMs,saveMs,peakJsHeapMiB:Math.round(peakHeap/1024/1024),maxHeartbeatGapMs:maxHeartbeatGap,errors};results.push(result);console.log(JSON.stringify(result));await page.close();
  }
  await fs.writeFile('target/browser-benchmarks.json',JSON.stringify(results,null,2));
} finally {await browser.close();}
