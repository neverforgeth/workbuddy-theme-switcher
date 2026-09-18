import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import fs from 'node:fs';
import {URL} from 'node:url';
import {createHash} from 'node:crypto';
import adapter from '../vendor/codedrobe/src/adapters/workbuddy.mjs';
import { buildProbeExpression } from '../vendor/codedrobe/src/runtime/renderer-payload.mjs';

export const composer = `<div class="cr-theme cr-input-box"><div class="cr-input-box__main"><div class="cr-input-container-wrapper"><div class="cr-input-container"><div class="cr-input-editor-host"><div role="textbox" contenteditable="true"></div></div><div class="cr-input-toolbar"><button class="cr-send-button">Send</button></div></div></div></div></div>`;
export const conversation = `<div id="root"><div class="teams-container"><aside class="conversation-sidebar"></aside><div class="teams-main-content"><div class="conversation-shell"><div class="conversation-shell__main"><div class="cr-theme conversation-timeline"><div class="cr-message-list-viewport"><div class="cr-message-list"><div class="cr-message-list__content"><div class="cr-document"><div class="cr-frame cr-frame--left"><div class="cr-frame__content"><div class="cr-markdown"><p>Public fixture</p><table><tbody><tr><td>Cell</td></tr></tbody></table><pre><code>const fixture = true</code></pre><blockquote>Quote</blockquote></div></div></div><div class="cr-frame cr-frame--right"><div class="cr-frame__content"><div class="cr-self-bubble">Question</div></div></div></div></div></div><div class="cr-message-list__bottom-mask"></div></div></div><div class="conversation-input-area">${composer}</div></div></div></div></div></div>`;
function probe(html) {
  const dom = new JSDOM(html, {runScripts:'outside-only'});
  dom.window.Element.prototype.getBoundingClientRect = () => ({width:300,height:90});
  try { return dom.window.eval(buildProbeExpression(adapter)); }
  finally { dom.window.close(); }
}
test('1.6.4: generic #root cannot pass as WorkBuddy', () => {
  assert.equal(probe('<div id="root"></div>').compatible, false);
});
test('1.6.4: new conversation activates required checks', () => {
  const result = probe(conversation);
  assert.equal(result.compatible, true);
  assert.ok(result.requirements.some(c => c.name === 'conversation-composer' && c.pass));
});
test('1.6.4: new conversation without editor fails closed', () => {
  assert.equal(probe(conversation.replace(composer, '')).compatible, false);
});
test('1.6.4: unknown chat shell cannot pass as an empty required set', () => {
  assert.equal(probe('<div id="root"><div class="teams-container"><div class="future-chat"></div></div></div>').compatible, false);
});
test('1.6.3 finished packages and resources remain byte-identical',()=>{
  const manifest=JSON.parse(fs.readFileSync(new URL('./builtin-1.6.3-sha256.json',import.meta.url),'utf8'));
  assert.ok(manifest.files.length>10);
  for(const file of manifest.files) assert.equal(createHash('sha256').update(fs.readFileSync(new URL('../'+file.path,import.meta.url))).digest('hex'),file.sha256,file.path);
});
