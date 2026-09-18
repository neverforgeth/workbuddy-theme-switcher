import { domProbe, paintProbe } from '../adapters/workbuddy-compat/index.mjs';

function safeHostClass(appId) {
  return `codedrobe-host-${String(appId).replace(/[^a-z0-9_-]/gi, "-")}`;
}

function resolveRendererProfile(adapter, targetTheme) {
  const profileId = targetTheme?.options?.rendererProfile;
  if (profileId === undefined) return null;
  if (typeof profileId !== "string" || !profileId.trim()) {
    throw new Error(`Theme renderer profile for '${adapter.id}' must be a non-empty string.`);
  }
  const profile = adapter.rendererProfiles?.[profileId];
  if (!profile || typeof profile.runtime !== "function") {
    throw new Error(`Adapter '${adapter.id}' does not support renderer profile '${profileId}'.`);
  }
  return profile;
}

function fallbackCleanupSource(adapter) {
  return Object.values(adapter.rendererProfiles ?? {})
    .filter((profile) => typeof profile.cleanup === "function")
    .map((profile) => `try { (${profile.cleanup.toString()})(); } catch {}`)
    .join("\n");
}

function buildCompatibilityProfile(adapter, themeVerification = null) {
  const adapterProfile = adapter.verification ?? { rootAny: ["body"], required: [] };
  const checks = (verification, scope, context = null) => [
    ...(verification?.required ?? []).map((item) => ({ ...item, scope, context, severity: "required" })),
    ...(verification?.recommended ?? []).map((item) => ({ ...item, scope, context, severity: "recommended" })),
  ];
  const contexts = (verification, scope) => (verification?.contexts ?? []).map((context) => ({
    name: context.name,
    scope,
    whenAny: context.when.any,
    checks: checks(context, scope, context.name),
  }));
  return {
    rootAny: adapterProfile.rootAny ?? ["body"],
    checks: [
      ...checks(adapterProfile, "adapter"),
      ...checks(themeVerification, "theme"),
    ],
    contexts: [
      ...contexts(adapterProfile, "adapter"),
      ...contexts(themeVerification, "theme"),
    ],
  };
}

function buildCompatibilityPrelude(adapter, themeVerification = null) {
  const profile = JSON.stringify(buildCompatibilityProfile(adapter, themeVerification));
  const appId = JSON.stringify(adapter.id);
  return `
    const appId = ${appId};
    const compatibilityProfile = ${profile};
    const inspect = (selector) => {
      try { return { selector, nodes: Array.from(document.querySelectorAll(selector)), valid: true, error: null }; }
      catch (error) { return { selector, nodes: [], valid: false, error: error?.message ?? String(error) }; }
    };
    const visible = (node) => {
      if (!node) return false;
      const box = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      return box.width > 0 && box.height > 0 && style.display !== 'none' && style.visibility !== 'hidden';
    };
    // A selector passes when ANY of its matches is visible: apps may keep
    // hidden duplicates (drawers, virtualized copies) ahead in DOM order.
    const evaluateSelectors = (selectors) => {
      const inspected = selectors.map(inspect);
      return {
        matches: inspected.filter((item) => item.valid && item.nodes.some(visible)).map((item) => item.selector),
        invalidSelectors: inspected.filter((item) => !item.valid).map((item) => ({ selector: item.selector, error: item.error })),
      };
    };
    const root = evaluateSelectors(compatibilityProfile.rootAny);
    const contexts = compatibilityProfile.contexts.map((context) => {
      const trigger = evaluateSelectors(context.whenAny);
      return {
        scope: context.scope,
        name: context.name,
        active: trigger.matches.length > 0,
        matches: trigger.matches,
        selectors: context.whenAny,
        invalidSelectors: trigger.invalidSelectors,
      };
    });
    const activeContexts = new Set(contexts.filter((context) => context.active).map((context) => context.scope + ':' + context.name));
    const checks = [
      ...compatibilityProfile.checks,
      ...compatibilityProfile.contexts.flatMap((context) => activeContexts.has(context.scope + ':' + context.name) ? context.checks : []),
    ];
    const requirements = checks.map((item) => {
      const evaluated = evaluateSelectors(item.any);
      return {
        scope: item.scope,
        context: item.context,
        severity: item.severity,
        name: item.name,
        pass: evaluated.matches.length > 0,
        matches: evaluated.matches,
        selectors: item.any,
        invalidSelectors: evaluated.invalidSelectors,
      };
    });
    const diagnostic = (item) => ({
      scope: item.scope,
      context: item.context,
      severity: item.severity,
      name: item.name,
      selectors: item.selectors,
      invalidSelectors: item.invalidSelectors,
    });
    const missing = [];
    const warnings = [];
    if (!root.matches.length) missing.push({
      scope: 'adapter', context: null, severity: 'required', name: 'root',
      selectors: compatibilityProfile.rootAny, invalidSelectors: root.invalidSelectors,
    });
    for (const item of requirements) {
      if (!item.pass && item.severity === 'required') missing.push(diagnostic(item));
      if (!item.pass && item.severity === 'recommended') warnings.push(diagnostic(item));
    }
    const hostContract = ${adapter.structuralContract ? domProbe : 'null'};
    if (hostContract) for (const check of hostContract.checks) {
      const item = {scope:'adapter',context:hostContract.scene,severity:'required',name:check.name,pass:check.pass,selectors:[],invalidSelectors:[]};
      requirements.push(item);
      if (!check.pass) missing.push(diagnostic(item));
    }
    const compatibility = {
      hostContract,
      appId,
      compatible: missing.length === 0,
      rootMatches: root.matches,
      rootInvalidSelectors: root.invalidSelectors,
      contexts,
      requirements,
      missing,
      warnings,
      viewport: { width: innerWidth, height: innerHeight },
    };`;
}

export function buildApplyExpression({ adapter, targetTheme }) {
  const profile = resolveRendererProfile(adapter, targetTheme);
  const host = JSON.stringify({ id: adapter.id, className: safeHostClass(adapter.id) });
  const theme = JSON.stringify(targetTheme.theme);
  const css = JSON.stringify(targetTheme.css);
  const images = JSON.stringify({
    ...(targetTheme.imageDataUrls ?? {}),
    ...(!targetTheme.imageDataUrls?.hero && targetTheme.artDataUrl ? { hero: targetTheme.artDataUrl } : {}),
  });
  const profileId = JSON.stringify(profile?.id ?? null);
  const profileFactory = profile ? `(${profile.runtime.toString()})` : "null";
  return `(() => {
    const host = ${host};
    const theme = ${theme};
    let cssText = ${css};
    const imageDataUrls = ${images};
    const profileId = ${profileId};
    const profileFactory = ${profileFactory};
    const rootState = window.__CODEDROBE__ ||= { hosts: {} };
    rootState.hosts ||= {};
    rootState.hosts[host.id]?.cleanup?.();
    const imageUrls = {};
    const imageBlobs = new Map();
    const ownedImageUrls = new Set();
    const resolveImageUrl = (dataUrl) => {
      if (!dataUrl?.startsWith('data:')) return null;
      try {
        const comma = dataUrl.indexOf(',');
        const mimeType = /^data:([^;,]+)/.exec(dataUrl)?.[1] || 'application/octet-stream';
        const binary = globalThis.atob(dataUrl.slice(comma + 1));
        const bytes = new Uint8Array(binary.length);
        for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
        const blob = new Blob([bytes], { type: mimeType });
        const objectUrl = globalThis.URL.createObjectURL(blob);
        imageBlobs.set(objectUrl, blob);
        ownedImageUrls.add(objectUrl);
        return objectUrl;
      } catch { /* Small data URLs remain a safe fallback when object URLs are unavailable. */ }
      return dataUrl;
    };
    for (const [name, dataUrl] of Object.entries(imageDataUrls)) {
      const imageUrl = resolveImageUrl(dataUrl);
      if (imageUrl && /^[a-z0-9][a-z0-9_-]*$/i.test(name)) imageUrls[name] = imageUrl;
    }
    const artDataUrl = imageDataUrls.hero ?? null;
    const artUrl = imageUrls.hero ?? null;
    let profileRuntime;
    try {
      profileRuntime = profileFactory ? profileFactory({
        theme, imageDataUrls, imageUrls, artDataUrl, artUrl,
      }) : null;
    } catch (error) {
      for (const objectUrl of ownedImageUrls) globalThis.URL?.revokeObjectURL?.(objectUrl);
      throw error;
    }
    const styleId = 'codedrobe-theme-style-' + host.id;

    const ensure = () => {
      const root = document.documentElement;
      if (!root) return false;
      root.classList.add('codedrobe-theme', host.className);
      root.dataset.codedrobeHost = host.id;
      root.dataset.codedrobeTheme = theme.id;
      root.dataset.codedrobeThemeVersion = theme.version;
      for (const [name, imageUrl] of Object.entries(imageUrls)) {
        root.style.setProperty('--codedrobe-image-' + name, 'url("' + imageUrl + '")');
      }
      if (artUrl) root.style.setProperty('--codedrobe-art', 'url("' + artUrl + '")');
      else root.style.removeProperty('--codedrobe-art');
      let style = document.getElementById(styleId);
      if (!style) {
        style = document.createElement('style');
        style.id = styleId;
        (document.head || root).appendChild(style);
      }
      if (style.dataset.themeVersion !== theme.id + '@' + theme.version) {
        style.textContent = cssText;
        style.dataset.themeVersion = theme.id + '@' + theme.version;
      }
      profileRuntime?.ensure?.();
      return true;
    };

    let timer;
    const observer = new MutationObserver(() => {
      clearTimeout(timer);
      timer = setTimeout(ensure, 120);
    });
    observer.observe(document.documentElement, { childList: true, subtree: true });
    const interval = setInterval(ensure, 5000);
    const cleanup = () => {
      observer.disconnect();
      clearTimeout(timer);
      clearInterval(interval);
      profileRuntime?.cleanup?.();
      for (const objectUrl of ownedImageUrls) globalThis.URL?.revokeObjectURL?.(objectUrl);
      ownedImageUrls.clear();
      document.getElementById(styleId)?.remove();
      const root = document.documentElement;
      root?.classList.remove(host.className);
      root?.style.removeProperty('--codedrobe-art');
      for (const name of Object.keys(imageUrls)) root?.style.removeProperty('--codedrobe-image-' + name);
      if (root?.dataset.codedrobeHost === host.id) {
        delete root.dataset.codedrobeHost;
        delete root.dataset.codedrobeTheme;
        delete root.dataset.codedrobeThemeVersion;
      }
      delete rootState.hosts[host.id];
      if (!Object.keys(rootState.hosts).length) root?.classList.remove('codedrobe-theme');
      return true;
    };
    rootState.hosts[host.id] = {
      cleanup, ensure, observer, interval,
      updateCss(expectedId, nextCss) {
        if (expectedId !== theme.id || typeof nextCss !== 'string' || nextCss.length > 500000) return false;
        cssText = nextCss;
        const style = document.getElementById(styleId);
        if (style) style.textContent = cssText;
        ensure();
        return true;
      },
      themeId: theme.id, version: theme.version,
      imageNames: Object.keys(imageUrls),
      imageReady: false,
      profileId, verifyProfile: profileRuntime?.verify ?? null,
    };
    const currentState=rootState.hosts[host.id];
    currentState.imageDecode=Promise.all(Object.values(imageUrls).map(src=>{
      const blob=imageBlobs.get(src);
      // Image.decode waits for a rendering opportunity in an occluded Electron window.
      // Decode the exact owned bytes without depending on foreground animation frames.
      if(blob && typeof globalThis.createImageBitmap==='function') return globalThis.createImageBitmap(blob).then(bitmap=>{
        const valid=bitmap.width>0&&bitmap.height>0;bitmap.close();if(!valid)throw new Error('Invalid image dimensions');
      });
      const image=new Image(); image.src=src; return image.decode();
    })).then(()=>{currentState.imageReady=true;return true;},()=>false).finally(()=>imageBlobs.clear());
    ensure();
    return { installed: true, appId: host.id, themeId: theme.id, version: theme.version };
  })()`;
}

export function buildRemoveExpression(adapter) {
  const appId = JSON.stringify(adapter.id);
  const hostClass = JSON.stringify(safeHostClass(adapter.id));
  const fallbackCleanup = fallbackCleanupSource(adapter);
  return `(() => {
    const appId = ${appId};
    const state = window.__CODEDROBE__?.hosts?.[appId];
    if (state?.cleanup) return state.cleanup();
    ${fallbackCleanup}
    document.getElementById('codedrobe-theme-style-' + appId)?.remove();
    const root = document.documentElement;
    root?.classList.remove(${hostClass});
    root?.style.removeProperty('--codedrobe-art');
    if (root?.style) {
      for (let index = root.style.length - 1; index >= 0; index -= 1) {
        const name = root.style.item(index);
        if (name.startsWith('--codedrobe-image-')) root.style.removeProperty(name);
      }
    }
    if (root?.dataset.codedrobeHost === appId) {
      delete root.dataset.codedrobeHost;
      delete root.dataset.codedrobeTheme;
      delete root.dataset.codedrobeThemeVersion;
    }
    if (root && ![...root.classList].some((name) => name.startsWith('codedrobe-host-'))) {
      root.classList.remove('codedrobe-theme');
    }
    return true;
  })()`;
}

export function buildProbeExpression(adapter, themeVerification = null) {
  return `(() => {
    ${buildCompatibilityPrelude(adapter, themeVerification)}
    return compatibility;
  })()`;
}

export function buildVerifyExpression(adapter, expectedTheme = null, themeVerification = null, targetTheme = null) {
  const profile = resolveRendererProfile(adapter, targetTheme);
  const expected = JSON.stringify(expectedTheme);
  const expectedProfileId = JSON.stringify(profile?.id ?? null);
  return `(async () => {
    ${buildCompatibilityPrelude(adapter, themeVerification)}
    const expected = ${expected};
    const expectedProfileId = ${expectedProfileId};
    const state = window.__CODEDROBE__?.hosts?.[appId];
    const imagesReady = state?.imageDecode ? await Promise.race([state.imageDecode,new Promise(resolve=>setTimeout(()=>resolve(false),2000))]) : true;
    const profile = state?.verifyProfile?.() ?? null;
    const coverage = ${adapter.structuralContract ? paintProbe : 'null'};
    const profileMissing = (profile?.missing ?? []).map((item) => ({
      scope: 'profile', context: profile.id ?? state?.profileId ?? null, severity: 'required',
      name: item.name, selectors: item.selectors ?? [], invalidSelectors: [],
    }));
    const result = {
      ...compatibility,
      installed: Boolean(state),
      themeId: state?.themeId ?? null,
      version: state?.version ?? null,
      stylePresent: Boolean(document.getElementById('codedrobe-theme-style-' + appId)),
      horizontalOverflow: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      images: state?.imageNames ?? [],
      profile,
      coverage,
    };
    result.missing = [...result.missing, ...profileMissing,...(coverage?.checks??[]).filter(c=>c.status==='failed').map(c=>({scope:'adapter',severity:'required',name:c.name,selectors:[],invalidSelectors:[]}))];
    const themeMatches = !expected || (result.themeId === expected.id && result.version === expected.version);
    const profileMatches = !expectedProfileId || (state?.profileId === expectedProfileId && profile?.pass === true);
    const actualStyle = document.getElementById('codedrobe-theme-style-' + appId);
    const cssMatches = ${targetTheme ? `actualStyle?.textContent === ${JSON.stringify(targetTheme.css)}` : 'true'};
    // Legacy packages can still be probed by the CLI. Full CR paint acceptance requires
    // the prepared runtime package; it is deliberately not inferred from a theme ID.
    const coveragePass = !coverage || coverage.status === 'passed';
    const gates = [
      ['theme-identity', themeMatches], ['css-identity', cssMatches],
      ['images-decoded', imagesReady], ['horizontal-layout', !result.horizontalOverflow],
      ['runtime-installed', result.installed], ['style-present', result.stylePresent],
      ['renderer-profile', profileMatches && (profile?.pass ?? true)],
    ];
    result.missing.push(...gates.filter(([,pass])=>!pass).map(([name])=>({scope:'adapter',severity:'required',name,selectors:[],invalidSelectors:[]})));
    result.pass = result.compatible && result.installed && result.stylePresent && themeMatches && cssMatches && coveragePass && imagesReady &&
      profileMatches && (profile?.pass ?? true) && !result.horizontalOverflow;
    return result;
  })()`;
}
