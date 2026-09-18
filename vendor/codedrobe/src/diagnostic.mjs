// The desktop host consumes only this bounded, versioned envelope. Never include
// exception messages, target titles/URLs, file paths, renderer text or CSS here.
export const DIAGNOSTIC_PREFIX = "[codedrobe-diagnostic] ";
const CODES = new Set([
  "CODEDROBE_RESTART_REQUIRED", "CODEDROBE_VERIFY_FAILED", "CODEDROBE_DOM_INCOMPATIBLE",
  "CODEDROBE_PORT_OCCUPIED", "CODEDROBE_TARGET_TIMEOUT", "CODEDROBE_CDP_TIMEOUT",
  "CODEDROBE_CDP_CONNECTION_FAILED", "CODEDROBE_CDP_PROTOCOL_ERROR",
  "CODEDROBE_RENDERER_EVALUATION_FAILED", "CODEDROBE_THEME_READ_FAILED",
  "CODEDROBE_THEME_INVALID", "TARGET_NOT_FOUND", "NOT_CONNECTED",
]);
const CHECKS = new Set([
  'home-canvas-paint',
  'theme-identity','css-identity','images-decoded','horizontal-layout','runtime-installed','style-present','renderer-profile',
  'host-root','known-scene','scene-composer','conversation-timeline','conversation-editor',
  'canvas-paint','composer-paint','editor-paint','editor-text','assistant-paint','assistant-text','user-paint','menu-paint','dialog-paint','send-paint','send-disc','send-hover','send-disc-hover','single-style-node',
  "root", "home-header", "home-composer", "home-composer-panel", "conversation-composer",
  "assistant-shell", "projects-shell", "expert-shell", "skills-shell", "connector-shell",
  "automation-shell", "project-chat-shell", "project-chat-composer",
]);
export function safeDiagnostic(error) {
  const missing = [
    ...(Array.isArray(error?.missing) ? error.missing : []),
    ...(Array.isArray(error?.results) ? error.results.flatMap(r => r.result?.missing ?? []) : []),
  ];
  return {
    version: 1,
    code: CODES.has(error?.code) ? error.code : "CODEDROBE_COMMAND_FAILED",
    checks: [...new Set(missing.map(r => r?.name).filter(name => CHECKS.has(name)))].sort(),
  };
}
export function codedError(code, message, cause) {
  return Object.assign(new Error(message, { cause }), { code });
}
