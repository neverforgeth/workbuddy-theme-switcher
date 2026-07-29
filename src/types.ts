export type Theme = {
  id: string;
  name: string;
  description: string;
  previewPath: string;
  verifiedWorkBuddyVersion: string;
  themeVersion: string;
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

export type OperationResult = {
  success: boolean;
  action: "apply" | "restore";
  themeId: string | null;
  message: string;
  rolledBack: boolean;
  styleNodeCount: number | null;
};

export type AppError = { code?: string; message?: string };
