/* global fetch, AbortSignal, URL, WebSocket, setTimeout, clearTimeout */
// Read-only acceptance probe. Collects geometry, never text, input values or screenshots.
import fs from "node:fs";
import process from "node:process";
import console from "node:console";
const label=process.argv[2];
if(!/^[a-z-]+$/.test(label||""))throw new Error("Pass a local report label");
const targets=await (await fetch("http://127.0.0.1:19336/json/list",{signal:AbortSignal.timeout(2000)})).json();
const target=targets.find(t=>t.type==="page"&&/workbuddy/i.test(t.url)&&/resources\/app.asar\/renderer\/index.html/i.test(t.url));
if(!target)throw new Error("No connected WorkBuddy renderer; will not start it");
const url=new URL(target.webSocketDebuggerUrl);
if(url.protocol!=="ws:"||!["127.0.0.1","localhost"].includes(url.hostname))throw new Error("Non-loopback CDP rejected");
const expression=`(() => { const selectors=['.teams-container','.conversation-sidebar','.teams-content-wrapper','.main-content','.chat-container',"[role='textbox'][contenteditable='true']","[class*='_iconBtnSend_']"]; return {theme:document.documentElement.dataset.codedrobeTheme??null,nodes:document.querySelectorAll('#codedrobe-theme-style-workbuddy').length,viewport:{width:innerWidth,height:innerHeight},geometry:selectors.flatMap(selector=>Array.from(document.querySelectorAll(selector)).slice(0,8).map((e,index)=>{const r=e.getBoundingClientRect(); const center=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);return {selector,index,width:r.width,height:r.height,scrollWidth:e.scrollWidth,scrollHeight:e.scrollHeight,pointerEvents:getComputedStyle(e).pointerEvents,hit:center!==null&&(e===center||e.contains(center))};}).filter(v=>v.width>0&&v.height>0))}; })()`;
const result=await new Promise((resolve,reject)=>{
  const socket=new WebSocket(url);
  const timeout=setTimeout(()=>{socket.close();reject(new Error("CDP deadline"));},3000);
  socket.addEventListener("open",()=>socket.send(JSON.stringify({id:1,method:"Runtime.evaluate",params:{expression,returnByValue:true}})));
  socket.addEventListener("error",()=>{clearTimeout(timeout);reject(new Error("CDP transport"));});
  socket.addEventListener("message",event=>{const message=JSON.parse(String(event.data));if(message.id!==1)return;clearTimeout(timeout);socket.close();if(message.error||message.result?.exceptionDetails)reject(new Error("Geometry evaluation failed"));else resolve(message.result.result.value);});
});
fs.mkdirSync(".qa",{recursive:true});
fs.writeFileSync(".qa/host-layout-"+label+".json",JSON.stringify(result,null,2));
console.log(JSON.stringify(result));
