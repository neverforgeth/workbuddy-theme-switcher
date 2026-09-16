/* global innerWidth, innerHeight, document, getComputedStyle, window, MutationObserver, scrollX, scrollY */
// No text values, attributes containing user data, or DOM snapshots leave this expression.
(() => {
  const rect = (r) => ({
    x: Math.max(0, r.left),
    y: Math.max(0, r.top),
    width: Math.max(0, Math.min(innerWidth, r.right) - Math.max(0, r.left)),
    height: Math.max(0, Math.min(innerHeight, r.bottom) - Math.max(0, r.top)),
  });
  const visible = (r) => r.width > 0 && r.height > 0;
  // Count changes, not text. A single observer replaces the old per-node upload-mask walk.
  const key = "__workbuddyStudioCaptureMarker";
  let marker = window[key];
  if (!marker) {
    marker = { epoch: 0 };
    const tick = () => {
      marker.epoch += 1;
    };
    marker.observer = new MutationObserver(tick);
    marker.observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      characterData: true,
      attributes: true,
    });
    document.addEventListener("scroll", tick, { capture: true, passive: true });
    document.addEventListener("input", tick, { capture: true, passive: true });
    window[key] = marker;
  }
  const entries = [
    ["sidebar", ".conversation-sidebar"],
    ["main", ".wb-home-page,.main-content--chat"],
    [
      "composer",
      ".wb-home-composer__input-slot>section,.chat-container section",
    ],
    ["userMessage", "[id^='user-message-'] [class*='_userMessageBubble_']"],
    ["assistantMessage", ".cb-assistant-message"],
    [
      "primaryButton",
      "button[type='submit'],.cb-button--primary,[class*='_iconBtnSend_']",
    ],
    ["secondaryButton", "button,[role='button']"],
    ["menu", "[role='menu'],[role='listbox'],[role='tooltip']"],
    ["dialog", "[role='dialog']"],
  ];
  const regions = entries.flatMap(([region, selector]) =>
    Array.from(document.querySelectorAll(selector))
      .slice(0, 80)
      .map((e) => {
        const r = rect(e.getBoundingClientRect()),
          s = getComputedStyle(e);
        return {
          region,
          rect: r,
          background: s.backgroundColor,
          foreground: s.color,
        };
      })
      .filter((e) => visible(e.rect)),
  );
  const scene = document.querySelector('[role="dialog"]')
    ? "dialog"
    : document.querySelector(".wb-home-page")
      ? "home"
      : document.querySelector(".main-content--chat")
        ? "chat"
        : "unknown";
  return {
    width: innerWidth,
    height: innerHeight,
    scene,
    regions,
    mutation_epoch: marker.epoch,
    scroll_x: scrollX,
    scroll_y: scrollY,
  };
})();
