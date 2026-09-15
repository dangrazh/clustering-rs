import { chromium,expect } from "@playwright/test";
import { mkdir } from "node:fs/promises";
await mkdir("target/ui-theme",{recursive:true});
const browser=await chromium.launch({headless:true});
const errors=[];
try {
  async function page(token,name){
    const mode=name==="Alice"?"light":"dark";const context=await browser.newContext({colorScheme:mode});
    if(!process.env.FIXTURE_EMAIL_LOGIN)await context.addCookies([{name:"app_session",value:token,url:process.env.FIXTURE_URL}]);
    const page=await context.newPage();page.on("pageerror",e=>errors.push(e.message));
    if(process.env.FIXTURE_EMAIL_LOGIN){
      await page.goto(`${process.env.FIXTURE_URL}/login`);
      await expect(page.locator('input')).toHaveCount(1);
      await page.screenshot({path:`target/ui-theme/login-${mode}.png`});
      const original=await page.locator("body").evaluate(e=>getComputedStyle(e).backgroundColor);await page.emulateMedia({colorScheme:mode==="light"?"dark":"light"});
      await expect.poll(()=>page.locator("body").evaluate(e=>getComputedStyle(e).backgroundColor)).not.toBe(original);await page.emulateMedia({colorScheme:mode});
      await page.getByLabel('Email',{exact:true}).fill(`${name.toLowerCase()}@example.invalid`);
      await page.getByRole('button',{name:'Sign in',exact:true}).click();
    }else await page.goto(process.env.FIXTURE_URL);
    await expect(page.locator("#signedInUser")).toHaveText(name);
    await page.getByRole("button",{name:"Analyses",exact:true}).click();
    await expect(page.getByRole("button",{name:"Browser acceptance",exact:true})).toBeVisible();
    const panelColor=await page.locator(".topbar").evaluate(e=>getComputedStyle(e).backgroundColor);
    await expect(page.locator("#analysisLibrary")).toHaveCSS("background-color",panelColor);await expect(page.locator(".collaboration-bar")).toHaveCSS("background-color",panelColor);
    await page.screenshot({path:`target/ui-theme/library-${mode}.png`});
    await page.getByRole("button",{name:"Browser acceptance",exact:true}).click();
    await expect(page.locator("#sharedTitle")).toHaveText("Browser acceptance");return page;
  }
  const alice=await page(process.env.FIXTURE_ALICE,"Alice"),bob=await page(process.env.FIXTURE_BOB,"Bob");
  for(const p of [alice,bob]){await p.locator('[data-workflow-key="1"]').click();await p.locator("#editLabel").click();}
  await alice.locator('#labelForm input').fill("Alice's label");await bob.locator('#labelForm input').fill("Bob's retained draft");
  await alice.locator('#labelForm button').first().click();
  await expect(alice.locator("#workflowPanel .workflow-label")).toHaveText("Alice's label");
  await expect(bob.locator('#labelForm input')).toHaveValue("Bob's retained draft");
  await bob.locator('#labelForm button').first().click();await expect(bob.locator("#workflowError")).toContainText("changed");
  await bob.locator('#labelForm button').first().click();await bob.getByRole("dialog",{name:"Review conflicting change"}).getByRole("button",{name:"Submit draft"}).click();
  await expect(bob.locator("#workflowPanel .workflow-label")).toHaveText("Bob's retained draft");
  await alice.locator("#closeWorkflow").click();await alice.locator('[data-workflow-key="1"]').click();
  await expect(alice.locator("#workflowPanel .workflow-label")).toHaveText("Bob's retained draft");
  await alice.locator('#commentForm textarea').fill("Alice's comment");await alice.locator('#commentForm button').click();
  await expect(alice.locator("#commentList")).toContainText("Alice's comment");
  await bob.locator("#closeWorkflow").click();await bob.locator('[data-workflow-key="1"]').click();
  await expect(bob.locator("#commentList")).toContainText("Alice's comment");await expect(bob.locator("#commentList [data-edit]")).toHaveCount(0);
  await bob.getByRole("button",{name:"Claim",exact:true}).click();
  await expect(bob.locator("#workflowPanel select").first()).toHaveValue(await bob.evaluate(async()=>{const {state}=await import('/state.js');return state.user.id;}));
  await bob.locator("#closeWorkflow").click();
  await bob.evaluate(async()=>{const {state}=await import('/state.js');state.pivotRows=[2];});
  await expect(bob.locator("#personalSaveStatus")).toHaveText("View saved");
  await bob.reload();await bob.getByRole("button",{name:"Analyses",exact:true}).click();await bob.getByRole("button",{name:"Browser acceptance",exact:true}).click();
  await expect.poll(()=>bob.evaluate(async()=>{const {state}=await import('/state.js');return state.pivotRows;})).toEqual([2]);
  await expect.poll(()=>alice.evaluate(async()=>{const {state}=await import('/state.js');return state.pivotRows;})).toEqual([]);
  await alice.locator("#commentForm textarea").fill("Retried once after lost acknowledgment");
  await alice.route("**/api/analyses/*/commands",async route=>{
    await route.fetch();await route.abort("failed");
  },{times:1});
  await alice.locator("#commentForm button").click();
  await expect(alice.locator("#commentList .comment-text").filter({hasText:"Retried once after lost acknowledgment"})).toHaveCount(1,{timeout:10000});
  // The first command commits, but its response is replaced with session expiry.
  // A different signed-in identity must not receive the retry or adopt the draft.
  let renewAsAlice=false;
  const originalIdentity=await alice.evaluate(async()=>{const {state}=await import('/state.js');return {user:state.user,csrf:state.csrf};});
  await alice.route("**/api/me",async route=>route.fulfill({json:renewAsAlice?originalIdentity:{user:{id:"another-user"},csrf:"wrong-user"}}));
  const renewalCommands=[];
  await alice.route("**/api/analyses/*/commands",async route=>{
    renewalCommands.push(route.request().postDataJSON().commandId);
    if(renewalCommands.length===1){await route.fetch();await route.fulfill({status:401,json:{error:"Session expired"}});}
    else await route.continue();
  });
  await alice.locator('#commentForm textarea').fill("Preserved through sign-in renewal");await alice.locator('#commentForm button').click();
  await expect(alice.locator("#sessionRenewal")).toContainText("different account");
  expect(renewalCommands.length).toBe(1);
  await alice.screenshot({path:"target/ui-theme/renewal-light.png"});
  renewAsAlice=true;
  await expect(alice.locator("#sessionRenewal")).toHaveCount(0);
  await expect(alice.locator("#commentList .comment-text").filter({hasText:"Preserved through sign-in renewal"})).toHaveCount(1);
  expect(renewalCommands.length).toBe(2);expect(renewalCommands[0]).toBe(renewalCommands[1]);
  await alice.unroute("**/api/me");await alice.unroute("**/api/analyses/*/commands");
  await alice.locator("#editLabel").click();await alice.locator("#labelForm input").fill("Keep during archive");
  const identity=await bob.evaluate(async()=>{const {state}=await import('/state.js');return {aid:state.analysisId,csrf:state.csrf,version:state.shared.analysis.version};});

  const archived=await bob.request.post(`${process.env.FIXTURE_URL}/api/analyses/${identity.aid}/commands`,{headers:{"X-CSRF-Token":identity.csrf},data:{commandId:crypto.randomUUID(),target:"",expected:identity.version,action:{type:"archive"}}});
  expect(archived.status()).toBe(200);const archiveState=await archived.json();
  for(const p of [alice,bob])await p.getByRole("dialog",{name:"Analysis update"}).getByRole("button",{name:"Close",exact:true}).click();
  await expect(alice.locator("#labelForm button").first()).toBeDisabled();await expect(alice.locator("#labelForm input")).toHaveValue("Keep during archive");
  const restored=await bob.request.post(`${process.env.FIXTURE_URL}/api/analyses/${identity.aid}/commands`,{headers:{"X-CSRF-Token":identity.csrf},data:{commandId:crypto.randomUUID(),target:"",expected:archiveState.snapshot.analysis.version,action:{type:"restore"}}});
  expect(restored.status()).toBe(200);await expect(alice.locator("#labelForm button").first()).toBeEnabled();
  const exported=await bob.request.post(`${process.env.FIXTURE_URL}/api/analyses/${identity.aid}/session/save`,{headers:{"X-CSRF-Token":identity.csrf},data:{}});expect(exported.status()).toBe(200);
  await alice.locator("#closeWorkflow").click();await alice.locator('[data-step="source"]').click();
  await alice.locator("#sessionInput").setInputFiles({name:"portable.icas",mimeType:"application/octet-stream",buffer:await exported.body()});
  await expect(alice.locator("#sharedTitle")).toContainText("Temporary analysis");
  await alice.locator("#centralSave").click();const saveDialog=alice.getByRole("dialog",{name:"Save analysis centrally"});await saveDialog.getByLabel("Analysis name").fill("Imported browser acceptance");await saveDialog.getByRole("button",{name:"Save",exact:true}).click();
  await expect(alice.locator("#sharedTitle")).toHaveText("Imported browser acceptance");
  await alice.locator('[data-workflow-key="1"]').click();await expect(alice.locator("#commentList [data-edit]")).toHaveCount(0);
  await expect(alice.locator("#workflowPanel")).toContainText("unconfirmed");
  for(const [p,mode] of [[alice,"light"],[bob,"dark"]]){
    if(await p.locator("#workflowDialog").isVisible())await p.locator("#closeWorkflow").click();
    await p.getByRole("button",{name:"My jobs",exact:true}).click();await expect(p.locator("#jobsDialog button").last()).toHaveText("Close");
    if(p===alice){
      await expect(p.locator("#jobsDialog .job-metadata").first()).toContainText("Source: browser-incidents.xlsx");
      const formatted = await p.evaluate(() => new Intl.NumberFormat().format(1200));
      await expect(p.locator("#jobsDialog .job-metadata").first()).toContainText(`Rows processed: ${formatted} of ${formatted}`);
      await expect(p.locator("#jobsDialog .job-metadata").first()).toContainText("Columns: 3");
    }
    await p.screenshot({path:`target/ui-theme/jobs-${mode}.png`});await p.locator("#jobsDialog").getByRole("button",{name:"Close",exact:true}).click();
  }
  if(errors.length)throw new Error(errors.join("\n"));
  console.log("PASS: two-browser edits, retained conflict draft, attribution, comment ownership, claiming, personal views, lost-ack retry, same-account sign-in renewal, archive/restore and portable import");
} finally {await browser.close();}
