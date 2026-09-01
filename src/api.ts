import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  PipelineState,
  Settings,
  StatusInfo,
  ToolReport,
} from "./types";

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  getStatus: (refresh: boolean) => invoke<StatusInfo>("get_status", { refresh }),
  checkTools: () => invoke<ToolReport>("check_tools"),
  provisionTools: () => invoke<void>("provision_tools"),
  startPipeline: () => invoke<void>("start_pipeline"),
  cancelPipeline: () => invoke<void>("cancel_pipeline"),
  retryPipeline: () => invoke<void>("retry_pipeline"),
  getPipelineState: () => invoke<PipelineState>("get_pipeline_state"),
  launchAseprite: () => invoke<void>("launch_aseprite"),
  uninstallAseprite: () => invoke<void>("uninstall_aseprite"),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),
  exportDiagnostics: () => invoke<string>("export_diagnostics"),
  copyToClipboard: (text: string) => invoke<void>("copy_to_clipboard", { text }),
  getAppVersion: () => invoke<string>("get_app_version"),
  checkAppUpdate: () => invoke<string | null>("check_app_update"),
  installAppUpdate: () => invoke<void>("install_app_update"),
  getLogTail: () => invoke<string[]>("get_log_tail"),
};

export function onPipelineState(cb: (s: PipelineState) => void): Promise<UnlistenFn> {
  return listen<PipelineState>("pipeline://state", (e) => cb(e.payload));
}

export function onPipelineLog(cb: (line: string) => void): Promise<UnlistenFn> {
  return listen<string>("pipeline://log", (e) => cb(e.payload));
}

export function onStatusChanged(cb: (s: StatusInfo) => void): Promise<UnlistenFn> {
  return listen<StatusInfo>("status://changed", (e) => cb(e.payload));
}
