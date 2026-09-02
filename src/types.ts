export type Channel = "stable" | "beta";
export type WatcherMode = "off" | "notify";

export interface Settings {
  channel: Channel;
  installDir: string; // empty string = platform default
  cleanupAfterBuild: boolean;
  trayEnabled: boolean;
  startMinimized: boolean;
  watcherMode: WatcherMode;
  watcherIntervalHours: number;
  parallelJobs: number; // 0 = auto
  checkOnLaunch: boolean;
  processWatch: boolean;
  useCcache: boolean;
  keepBuildFiles: boolean;
}

export interface ToolStatus {
  id: string;
  name: string;
  ok: boolean;
  detail: string;
  provisionable: boolean;
}

export interface ToolReport {
  tools: ToolStatus[];
  allOk: boolean;
  helperCommand: string | null;
  helperLabel: string | null;
}

export interface StatusInfo {
  installedVersion: string | null;
  installedPath: string | null;
  latestVersion: string | null;
  latestName: string | null;
  lastCheck: number | null; // epoch millis
  busy: boolean;
}

export type StageStatus = "pending" | "running" | "done" | "failed" | "skipped";

export interface StageInfo {
  id: string;
  name: string;
  status: StageStatus;
  progress: number | null; // 0..1, null = indeterminate
  detail: string;
}

export interface BuildSummary {
  version: string;
  elapsedSecs: number;
  installedBytes: number;
  cleanedBytes: number;
}

export interface PipelineState {
  running: boolean;
  stages: StageInfo[];
  error: string | null;
  failedStage: string | null;
  summary: BuildSummary | null;
}
