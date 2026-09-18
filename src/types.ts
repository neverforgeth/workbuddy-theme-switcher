export type ThemeRef = { id: string; revision: number | null };
export type ThemeControls = {
  brightness: number;
  blur: number;
  panelOpacity: number;
  accent: string | null;
};
export type Palette = {
  background: string;
  surface: string;
  surfaceStrong: string;
  sidebar: string;
  accent: string;
  accentStrong: string;
  onAccent: string;
  border: string;
  text: string;
  textMuted: string;
  dark: boolean;
};
export type ContrastCheck = {
  name: string;
  ratio: number;
  required: number;
  status: "pass" | "warning" | "unverified";
};
export type CompiledTheme = {
  css: string;
  palette: Palette;
  checks: ContrastCheck[];
  templateVersion: string;
  effectiveOpacity: number;
  compileMs: number;
  effectiveDesign?: DesignAdvice | null;
  corrections?: string[];
};
export type ThemeDocument = {
  schemaVersion: number;
  draftId: string;
  themeId: string | null;
  revision: number | null;
  name: string;
  assetId: string;
  sourceFilename: string;
  width: number;
  height: number;
  background: string;
  extractedAccent: string;
  controls: ThemeControls;
  advice: Record<string, string | null>;
  compiled: CompiledTheme;
  editSequence: number;
  savedSequence: number | null;
  design?: DesignAdvice | null;
  backgroundSamples?: string[];
  style?: StyleSelection | null;
  analysis?: {
    version: number;
    recommended: StyleId;
    samples: string[];
    candidates: Array<{ color: string; area: number; score: number }>;
    luminance: number[];
    warmth: number;
    grid?: {
      width: number;
      height: number;
      mean: number;
      deviation: number;
      maxNeighborDelta: number;
      saturation: number;
    } | null;
  } | null;
  manualOverrides?: {
    regions: Partial<Record<Region, RegionStyle>>;
    global: boolean;
    position: boolean;
  };
};
export type DraftView = {
  document: ThemeDocument;
  imagePath: string;
  warning: string | null;
};
export type StyleId = "airy-light" | "warm-paper" | "calm-dark";
export type StyleSelection = { id: StyleId; version: number };
export type DraftUpdate = {
  name: string;
  controls: ThemeControls;
  sequence: number;
  style?: StyleSelection | null;
  resetStyle?: boolean;
  design?: DesignAdvice | null;
};
export type LibraryItem = {
  reference: ThemeRef;
  name: string;
  description: string;
  previewPath: string;
  editable: boolean;
  isCustom: boolean;
  palette: Palette | null;
  compatibility: string;
};
export type ThemePreview = {
  name: string;
  css: string;
  imagePath: string;
  compiled: CompiledTheme | null;
};
export type WorkBuddyStatus = {
  appFound: boolean;
  path: string | null;
  version: string | null;
  running: boolean;
  cdpAvailable: boolean;
  rendererAvailable: boolean;
  currentThemeId: string | null;
  styleNodeCount: number | null;
  logsDirectory: string;
  message: string;
};
export type RuntimeStatus = {
  desiredState: "theme" | "original";
  selectedThemeId: string | null;
  autoKeepTheme: boolean;
  monitorStatus: string;
  retryCount: number;
  lastAutoRecoveryAt: string | null;
  lastErrorCode: string | null;
  statePath: string;
  loginAutostartEnabled: boolean;
};
export type TrialSession = {
  id: string;
  name: string;
  phase: "preparing" | "active" | "restoring" | "pendingRecovery";
  deadlineMs: number;
  error: string | null;
  syncedSequence?: number | null;
  cssHash?: string | null;
};
export type Region =
  | "sidebar"
  | "main"
  | "composer"
  | "userMessage"
  | "assistantMessage"
  | "primaryButton"
  | "secondaryButton"
  | "menu"
  | "dialog";
export type RegionStyle = {
  background: string;
  text: string;
  muted: string;
  border: string;
  hover: string;
  selected: string;
  focusRing: string;
  opacity: number;
  shadow: "none" | "soft" | "raised";
};
export type DesignAdvice = {
  regions: Record<Region, RegionStyle>;
  backgroundX: number;
  backgroundY: number;
  veil: number;
  explanation: string;
};
export type Rect = { x: number; y: number; width: number; height: number };
export type RenderedPreview = {
  id: string;
  trialId: string;
  draftId: string;
  sequence: number;
  cssHash: string;
  targetId: string;
  width: number;
  height: number;
  scene: string;
  capturedAt: string;
  workbuddyVersion: string | null;
  imageDataUrl: string;
  captureMs: number;
};
export type RuntimeSnapshot = {
  compatibility?: {adapterVersion:string;structure:string;scene:string;status:string;code:string;identity:boolean;checkedAt:string;durationMs:number;checks:{name:string;status:string;pass:boolean}[];unchecked:string[]} | null;
  workbuddy: WorkBuddyStatus;
  runtime: RuntimeStatus;
  trial: TrialSession | null;
  operation: string | null;
  capturedAt: string;
  error: string | null;
};
export type OperationResult = {
  success: boolean;
  action: string;
  themeId: string | null;
  message: string;
  rolledBack: boolean;
  styleNodeCount: number | null;
};
export type AppError = { code?: string; message?: string };
