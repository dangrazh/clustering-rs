import { chromium, expect } from "@playwright/test";
const browser = await chromium.launch({headless:true});
try {
  const context = await browser.newContext();
  await context.addCookies([{name:"app_session",value:process.env.FIXTURE_ALICE,url:process.env.FIXTURE_URL}]);
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(process.env.FIXTURE_URL);
  await expect(page.locator("#signedInUser")).toHaveText("Alice");
  await page.getByRole("button",{name:"Analyses",exact:true}).click();
  const started = performance.now();
  await page.getByRole("button",{name:"Browser acceptance",exact:true}).click();
  await expect(page.locator("#analysisLibrary")).not.toBeVisible({timeout:60000});
  await expect(page.locator('[data-workflow-key="1"]')).toBeVisible({timeout:60000});
  const elapsedMs = Math.round(performance.now() - started);
  const measurements = await page.evaluate(async () => {
    const {state} = await import('/state.js');
    return {rows:state.analysis.source.rows.length,columns:state.analysis.source.headers.length,jsHeapBytes:performance.memory?.usedJSHeapSize};
  });
  expect(measurements.rows).toBe(150000); expect(measurements.columns).toBe(30);
  expect(errors).toEqual([]);
  console.log(JSON.stringify({ ...measurements, browserReadyMs:elapsedMs, transport:"local HTTP", browser:browser.version() }));
  expect(elapsedMs).toBeLessThan(30000);
} finally { await browser.close(); }
