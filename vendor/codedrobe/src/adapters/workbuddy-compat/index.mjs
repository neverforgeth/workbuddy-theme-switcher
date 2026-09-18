import fs from 'node:fs';
import { createHash } from 'node:crypto';
const read = name => fs.readFileSync(new URL(name, import.meta.url), 'utf8');
export const domProbe = read('dom.js');
export const paintProbe = read('paint.js').replace('__WORKBUDDY_DOM__',domProbe);
export const adapterVersion = 'workbuddy-164.1';
const builtin = read('builtin.css'), image = read('image.css'), components = read('components.css');
const hash = value => createHash('sha256').update(value).digest('hex');
export function effectiveTheme(css, structure) {
  let finalCss = css;
  if (structure === 'cr-v1') {
    const aliases = css.includes('--fusion-main-bg:') ? image : css.includes('--wb-theme-surface-composer-rgb:') ? builtin : null;
    if (!aliases) throw Object.assign(new Error('Unsupported palette contract'), {code:'CODEDROBE_PALETTE_UNSUPPORTED'});
    finalCss += '\n/* '+adapterVersion+' */\n'+aliases+'\n'+components;
  }
  return {originalHash:hash(css),adapterVersion,structure,css:finalCss,finalHash:hash(finalCss)};
}
