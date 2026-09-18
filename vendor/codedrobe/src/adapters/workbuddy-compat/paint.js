/* No page content or CSS is returned. A check can only be passed, failed, or unchecked. */
(() => {
  const contract = __WORKBUDDY_DOM__;
  const checks = contract.checks.map(c=>({...c,status:c.pass?'passed':'failed'}));
  const root = getComputedStyle(document.documentElement);
  const canvas = new OffscreenCanvas(1,1).getContext('2d',{willReadFrequently:true});
  const rgba = color => {
    if (!color?.trim() || !CSS.supports('color',color)) return null;
    canvas.clearRect(0,0,1,1);canvas.fillStyle=color;canvas.fillRect(0,0,1,1);
    return [...canvas.getImageData(0,0,1,1).data];
  };
  const same = (a,b) => a && b && a.every((v,i)=>Math.abs(v-b[i])<=1);
  const check = (name,selector,property,token) => {
    const elements = [...document.querySelectorAll(selector)].filter(el=>{
      const r=el.getBoundingClientRect(),s=getComputedStyle(el);
      return r.width>0&&r.height>0&&s.display!=='none'&&s.visibility!=='hidden';
    });
    const expected = rgba(token.startsWith('--')?root.getPropertyValue(token):token);
    const pass = elements.length>0 && elements.every(el=>same(rgba(getComputedStyle(el)[property]),expected));
    checks.push({name,pass,status:elements.length?(pass?'passed':'failed'):'unchecked'});
  };
  if (contract.structure==='cr-v1') {
    check('canvas-paint','.conversation-shell','backgroundColor','--wb164-canvas');
    check('home-canvas-paint','.wb-home-route','backgroundColor','transparent');
    check('composer-paint','.cr-input-container','backgroundColor','--wb164-composer');
    check('editor-paint','.cr-input-editor-host [contenteditable="true"]','backgroundColor','transparent');
    check('editor-text','.cr-input-editor-host [contenteditable="true"]','color','--wb164-composer-text');
    check('assistant-paint','.cr-frame--left>.cr-frame__content','backgroundColor','--wb164-assistant');
    check('assistant-text','.cr-frame--left .cr-markdown','color','--wb164-assistant-text');
    check('user-paint','.cr-self-bubble','backgroundColor','--wb164-user');
    check('menu-paint',contract.selectors.menu,'backgroundColor','--wb164-menu');
    check('dialog-paint',contract.selectors.dialog,'backgroundColor','--wb164-dialog');
    check('send-paint','.cr-send-button:not(:disabled):not(:hover):not(:has(path[fill-rule="evenodd"]))','backgroundColor','--wb164-primary');
    check('send-hover','.cr-send-button:not(:disabled):hover:not(:has(path[fill-rule="evenodd"]))','backgroundColor','--wb164-hover');
    check('send-disc','.cr-send-button:not(:disabled):not(:hover):has(path[fill-rule="evenodd"])','color','--wb164-primary');
    check('send-disc-hover','.cr-send-button:not(:disabled):hover:has(path[fill-rule="evenodd"])','color','--wb164-hover');
  }
  const count = document.querySelectorAll('#codedrobe-theme-style-workbuddy').length;
  checks.push({name:'single-style-node',pass:count===1,status:count===1?'passed':'failed'});
  const pass=checks.every(c=>c.status!=='failed');
  return {adapterVersion:contract.adapterVersion,structure:contract.structure,scene:contract.scene,
    status:pass?'passed':'failed',checks,unchecked:[...contract.unchecked,...checks.filter(c=>c.status==='unchecked').map(c=>c.name),'unobserved-interaction-states',...(contract.structure==='legacy'?['legacy-visual-coverage']:[])],checkedAt:new Date().toISOString(),
    code:pass?'OK':contract.compatible?'COMPAT_STYLE_MISMATCH':'COMPAT_STRUCTURE_UNSUPPORTED'};
})()
