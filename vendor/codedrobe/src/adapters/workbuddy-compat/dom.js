/* Read-only structural contract, shared verbatim by Rust, CodeDrobe and captures.
   Never return text, input values, account attributes, URLs or DOM snapshots. */
(() => {
  const selectors = {
    sidebar: '.conversation-sidebar,.conversation-list',
    main: '.conversation-shell,.main-content--chat,.chat-container,.wb-home-page,.project-detail-view',
    composer: '.cr-input-container,.wb-home-composer__input-slot>section,.chat-container section,.project-detail-view [class*="_mainArea_"]',
    editor: '[role="textbox"][contenteditable="true"],textarea',
    userMessage: '.cr-self-bubble,[id^="user-message-"] [class*="_userMessageBubble_"]',
    assistantMessage: '.cr-frame--left>.cr-frame__content,.cb-assistant-message',
    primaryButton: '.cr-send-button,button[type="submit"],.cb-button--primary,[class*="_iconBtnSend_"]',
    secondaryButton: 'button:not(.cr-send-button),[role="button"]',
    menu: '.cr-popover,.cr-sub-menu__overlay,.cr-model-selector__popover,[role="menu"],[role="listbox"]',
    dialog: '[role="dialog"]',
  };
  const nodes = selector => [...document.querySelectorAll(selector)].filter(el => {
    const r = el.getBoundingClientRect(), s = getComputedStyle(el);
    return r.width > 0 && r.height > 0 && s.display !== 'none' && s.visibility !== 'hidden';
  });
  const has = selector => nodes(selector).length > 0;
  const modern = has('.conversation-shell,.cr-input-container');
  const chat = has('.conversation-shell,.main-content--chat,.chat-container:not(.chat-container--welcome)');
  const home = has('.wb-home-page,.chat-container--welcome');
  const project = has('.project-detail-view');
  const workspace=has('.claw-workspace,.workbuddy-collab:not(.workbuddy-collab--detail),.expert-center-page,.skills-view,.connector-panel,.automation-main-page');
  const scene = chat ? 'chat' : home ? 'home' : project ? 'project' : workspace ? 'workspace' : 'unknown';
  const checks = [{name:'host-root',pass:has('.teams-container')}, {name:'known-scene',pass:scene !== 'unknown'}];
  if (chat||home||project) checks.push({name:chat?'conversation-composer':'scene-composer',pass:has(selectors.composer)});
  if (has('.conversation-shell')) {
    checks.push({name:'conversation-timeline',pass:has('.conversation-shell .conversation-timeline .cr-message-list-viewport')});
    checks.push({name:'conversation-editor',pass:has('.conversation-input-area [contenteditable="true"]')});
  }
  return {adapterVersion:'workbuddy-164.1',structure:modern?'cr-v1':scene==='unknown'?'unknown':'legacy',scene,
    checks,compatible:checks.every(c=>c.pass),selectors,
    unchecked:['menu','dialog','table','code'].filter(k=>!has({menu:selectors.menu,dialog:selectors.dialog,table:'.cr-markdown table,.cb-markdown table',code:'.cr-markdown pre,.cb-markdown pre'}[k]))};
})()
