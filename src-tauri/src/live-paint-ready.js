/* global document, requestAnimationFrame, cancelAnimationFrame, setTimeout, clearTimeout */
// Hidden Electron renderers can suspend RAF indefinitely. Never let that hold the host lock.
new Promise((resolve) => {
  let first, second;
  const done = (painted) => {
    cancelAnimationFrame(first);
    cancelAnimationFrame(second);
    clearTimeout(timer);
    resolve(painted);
  };
  const timer = setTimeout(() => done(false), 650);
  if (document.hidden) {
    done(false);
    return;
  }
  first = requestAnimationFrame(() => {
    second = requestAnimationFrame(() => done(true));
  });
});
