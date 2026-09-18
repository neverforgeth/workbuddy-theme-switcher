import assert from 'node:assert/strict';
import test,{before,after} from 'node:test';
import fs from 'node:fs';
import {chromium} from '@playwright/test';
import {conversation,composer} from './workbuddy-164.test.mjs';
import {URL} from 'node:url';
import {effectiveTheme,paintProbe,domProbe} from '../vendor/codedrobe/src/adapters/workbuddy-compat/index.mjs';
import {resolveThemeTarget} from '../vendor/codedrobe/src/theme/package.mjs';
import {buildApplyExpression,buildRemoveExpression,buildVerifyExpression} from '../vendor/codedrobe/src/runtime/renderer-payload.mjs';
import {officialCss} from './workbuddy-official-fixture.mjs';
import adapter from '../vendor/codedrobe/src/adapters/workbuddy.mjs';
let browser,page;
const catalog=JSON.parse(fs.readFileSync(new URL('../themes.json',import.meta.url),'utf8'));
const baseCss=officialCss();
// Host shell rules transcribed from the signed official ui-docs-viewer CSS.
const layout=`html,body{margin:0} .teams-container{display:flex;height:820px}.conversation-sidebar{width:240px;flex:none}.teams-main-content{flex:1;min-width:0}.conversation-shell{display:flex;flex-direction:column;height:100%;min-height:0;background:var(--wb-home-bg-secondary,#fafafa);overflow:hidden;position:relative}.conversation-shell__main{position:relative;display:flex;flex-direction:column;flex:1;min-width:0;min-height:0;overflow:hidden}.conversation-timeline{flex:1;min-height:0}.cr-message-list-viewport{height:400px}.cr-input-editor-host [contenteditable]{min-height:80px}`;
before(async()=>{browser=await chromium.launch({channel:'msedge',headless:true});page=await browser.newPage({viewport:{width:1200,height:820}});await page.route('**/*',route=>route.abort());});
after(async()=>{await browser?.close();});
async function mount(css){await page.setContent(`<style>${baseCss}\n${layout}</style>${conversation}`);await page.evaluate(buildApplyExpression({adapter,targetTheme:{theme:{id:'fixture',version:'1'},css}}));}
for(const record of catalog) test(`finished ${record.id}: new chat paint, one node, dimensions preserved`,async()=>{
  const pkg=JSON.parse(fs.readFileSync(new URL('../'+record.packagePath,import.meta.url),'utf8'));
  const target=resolveThemeTarget(pkg,'workbuddy');
  await page.setContent(`<style>${baseCss}\n${layout}</style>${conversation}`);
  await page.evaluate(buildApplyExpression({adapter,targetTheme:target}));
  const baseline=await page.locator('.cr-input-container').boundingBox();
  const effective=effectiveTheme(target.css,'cr-v1');
  await page.evaluate(buildApplyExpression({adapter,targetTheme:{...target,css:effective.css}}));
  const report=await page.evaluate(paintProbe);
  assert.equal(report.status,'passed',JSON.stringify(report.checks.filter(c=>c.status==='failed')));
  assert.deepEqual(await page.locator('.cr-input-container').boundingBox(),baseline);
  assert.equal(await page.locator('#codedrobe-theme-style-workbuddy').count(),1);
  await page.evaluate(buildRemoveExpression(adapter));assert.equal(await page.locator('#codedrobe-theme-style-workbuddy').count(),0);
  assert.equal(effectiveTheme(target.css,'legacy').css,target.css);
});
test('negative: missing overlay and overwritten paint do not pass',async()=>{
  const pkg=JSON.parse(fs.readFileSync(new URL('../'+catalog[0].packagePath,import.meta.url),'utf8'));
  const source=pkg.targets.workbuddy.css;
  await mount(source);assert.equal((await page.evaluate(paintProbe)).status,'failed');
  await mount(effectiveTheme(source,'cr-v1').css);
  await page.locator('.conversation-shell').evaluate(el=>el.style.setProperty('background','white','important'));
  assert.equal((await page.evaluate(paintProbe)).status,'failed');
});
test('real 5.5.6 home route cannot hide wallpaper behind an opaque white shell',async()=>{
  const home=`<div class="teams-container"><div class="teams-content-wrapper"><div class="teams-main-content"><main class="wb-home-route"><div class="workbuddy-topbar"></div><div class="wb-home-route__body"><div class="wb-home-page"><header class="wb-home-header">Public fixture</header><div class="wb-home-composer"><div class="wb-home-composer__input-slot">${composer}</div></div></div></div></main></div></div></div>`;
  const targets=catalog.map(record=>resolveThemeTarget(JSON.parse(fs.readFileSync(new URL('../'+record.packagePath,import.meta.url),'utf8')),'workbuddy'));
  targets.push(...JSON.parse(fs.readFileSync(new URL('../.qa/engine-fixtures.json',import.meta.url),'utf8')).map(f=>({theme:{id:'fixture',version:'1'},css:f.view.document.compiled.css})));
  for(const target of targets) {
    await page.setContent(`<style>${baseCss}\n${layout}\n.wb-home-route{background:#fff}</style>${home}`);
    await page.evaluate(buildApplyExpression({adapter,targetTheme:{...target,css:effectiveTheme(target.css,'cr-v1').css}}));
    assert.equal(await page.locator('.wb-home-route').evaluate(el=>el.ownerDocument.defaultView.getComputedStyle(el).backgroundColor),'rgba(0, 0, 0, 0)',target.theme.id);
    await page.locator('.wb-home-route').evaluate(el=>el.style.setProperty('background','white','important'));
    assert.ok((await page.evaluate(paintProbe)).checks.some(c=>c.name==='home-canvas-paint'&&c.status==='failed'));
    await page.evaluate(buildRemoveExpression(adapter));
  }
});
test('new chat has a scene and screenshot region selectors',async()=>{
  await page.setContent(`<style>${baseCss}\n${layout}</style>${conversation}`);
  const contract=await page.evaluate(domProbe);assert.equal(contract.scene,'chat');assert.equal(contract.structure,'cr-v1');
  assert.ok(await page.locator(contract.selectors.composer).count());
  await page.locator('.conversation-input-area').evaluate(el=>el.innerHTML='');
  assert.equal((await page.evaluate(domProbe)).compatible,false);
});
test('15 image/style palettes map to their own effective regions',async()=>{
  const fixtures=JSON.parse(fs.readFileSync(new URL('../.qa/engine-fixtures.json',import.meta.url),'utf8')).filter(f=>f.view.document.compiled.css.includes('--fusion-main-bg:'));
  assert.ok(fixtures.length>=15);
  for(const fixture of fixtures){
    const effective=effectiveTheme(fixture.view.document.compiled.css,'cr-v1');await mount(effective.css);
    const report=await page.evaluate(paintProbe);
    assert.equal(report.status,'passed',fixture.name+JSON.stringify(report.checks.filter(c=>c.status==='failed')));
    assert.ok(!effective.css.includes('--cg-text'));
  }
});
test('all palettes: menu/dialog, home, button states, input and scroll',async()=>{
  const fixtures=JSON.parse(fs.readFileSync(new URL('../.qa/engine-fixtures.json',import.meta.url),'utf8')).filter(f=>f.view.document.compiled.css.includes('--fusion-main-bg:'));
  for(const fixture of fixtures) {
    const target={theme:{id:'fixture',version:'1'},css:effectiveTheme(fixture.view.document.compiled.css,'cr-v1').css,imageDataUrls:{hero:fixture.view.imagePath}};
    const home=`<div id="root"><div class="teams-container"><div class="teams-content-wrapper"><main class="wb-home-page"><header class="wb-home-header">Welcome</header><div class="wb-home-composer"><div class="wb-home-composer__input-slot">${composer}</div></div></main></div></div></div>`;
    for(const html of [conversation,home]) {
      await page.setContent(`<style>${baseCss}\n${layout}</style>${html}`);
      await page.evaluate(buildApplyExpression({adapter,targetTheme:target}));
      const verified=await page.evaluate(buildVerifyExpression(adapter,target.theme,null,target));
      assert.equal(verified.pass,true,fixture.name+JSON.stringify(verified.missing));
      await page.locator('[contenteditable]').fill('Unsubmitted fixture');
      await page.locator('.cr-send-button').hover();
      const hovered=await page.evaluate(paintProbe);assert.equal(hovered.status,'passed',fixture.name+JSON.stringify(hovered.checks.filter(c=>c.status==='failed')));
      await page.locator('.cr-send-button').evaluate(el=>{el.disabled=true;});
      assert.equal((await page.evaluate(paintProbe)).status,'passed');
      await page.locator('body').evaluate(el=>el.insertAdjacentHTML('beforeend','<div class="cr-theme cr-popover" role="menu">Menu</div><div class="cr-theme" role="dialog">Dialog</div>'));
      assert.equal((await page.evaluate(paintProbe)).status,'passed',fixture.name+' popup');
      await page.evaluate("window.__CODEDROBE__.hosts.workbuddy.updateCss('fixture',document.getElementById('codedrobe-theme-style-workbuddy').textContent+'\\n/* edit */')");
      assert.equal(await page.locator('[contenteditable]').textContent(),'Unsubmitted fixture');
      assert.equal(await page.locator('#codedrobe-theme-style-workbuddy').count(),1);
    }
  }
});
test('negative: broken image, duplicate node and exact CSS drift fail verification',async()=>{
  const pkg=JSON.parse(fs.readFileSync(new URL('../'+catalog[0].packagePath,import.meta.url),'utf8'));
  const target=resolveThemeTarget(pkg,'workbuddy');target.css=effectiveTheme(target.css,'cr-v1').css;
  await mount(target.css);
  const check=()=>page.evaluate(buildVerifyExpression(adapter,target.theme,null,target));
  await page.evaluate(buildApplyExpression({adapter,targetTheme:{...target,imageDataUrls:{hero:'data:image/png;base64,aW52YWxpZA=='}}}));
  const broken=await check();assert.equal(broken.pass,false);
  assert.ok(broken.missing.some(c=>c.name==='images-decoded'));
  await page.evaluate(buildApplyExpression({adapter,targetTheme:target}));
  assert.equal((await check()).pass,true);
  await page.locator('#codedrobe-theme-style-workbuddy').evaluate(el=>el.after(el.cloneNode(true)));
  assert.equal((await check()).pass,false);
  await page.locator('#codedrobe-theme-style-workbuddy').last().evaluate(el=>el.remove());
  await page.locator('#codedrobe-theme-style-workbuddy').evaluate(el=>el.textContent+='\n/*external*/');
  const drifted=await check();assert.equal(drifted.pass,false);
  assert.ok(drifted.missing.some(c=>c.name==='css-identity'));
});
test('background resource validation does not wait for foreground Image.decode paint',async()=>{
  const target=resolveThemeTarget(JSON.parse(fs.readFileSync(new URL('../'+catalog[0].packagePath,import.meta.url),'utf8')),'workbuddy');target.css=effectiveTheme(target.css,'cr-v1').css;
  await page.setContent(`<style>${baseCss}\n${layout}</style>${conversation}`);
  await page.evaluate(()=>{globalThis.qaOriginalDecode=globalThis.Image.prototype.decode;globalThis.Image.prototype.decode=()=>new Promise(()=>{});});
  try {
    await page.evaluate(buildApplyExpression({adapter,targetTheme:target}));
    const result=await page.evaluate(buildVerifyExpression(adapter,target.theme,null,target));
    assert.equal(result.pass,true,JSON.stringify(result.missing));
  } finally {await page.evaluate(()=>{globalThis.Image.prototype.decode=globalThis.qaOriginalDecode;delete globalThis.qaOriginalDecode;});await page.evaluate(buildRemoveExpression(adapter));}
});
