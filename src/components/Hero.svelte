<script lang="ts">
  import { api } from "../api";
  import type { PipelineState, StatusInfo } from "../types";

  let {
    status,
    pipeline,
    onRefresh,
  }: {
    status: StatusInfo | null;
    pipeline: PipelineState | null;
    onRefresh: () => Promise<void>;
  } = $props();

  let checking = $state(false);
  let actionError = $state<string | null>(null);

  const installed = $derived(status?.installedVersion ?? null);
  const latest = $derived(status?.latestVersion ?? null);
  const upToDate = $derived(installed !== null && latest !== null && installed === latest);
  const running = $derived(pipeline?.running ?? false);

  const primaryLabel = $derived.by(() => {
    if (!installed) return latest ? `Install ${latest}` : "Install";
    if (!upToDate && latest) return `Update to ${latest}`;
    return "Rebuild";
  });

  async function start() {
    actionError = null;
    try {
      await api.startPipeline();
    } catch (e) {
      actionError = String(e);
    }
  }

  async function cancel() {
    try {
      await api.cancelPipeline();
    } catch (e) {
      actionError = String(e);
    }
  }

  async function launch() {
    actionError = null;
    try {
      await api.launchAseprite();
    } catch (e) {
      actionError = String(e);
    }
  }

  async function refresh() {
    checking = true;
    actionError = null;
    try {
      await onRefresh();
    } catch (e) {
      actionError = String(e);
    } finally {
      checking = false;
    }
  }
</script>

<section class="hero card">
  <div class="row">
    <div class="left">
      <div class="section-label">Aseprite</div>
      <div class="version-line">
        <span class="version">{installed ?? "Not installed"}</span>
        {#if installed && upToDate}
          <span class="status ok">Up to date</span>
        {:else if latest}
          <span class="status update">{latest} available</span>
        {/if}
      </div>
      <div class="meta dim">
        {#if status?.installedPath}
          <button class="link" onclick={() => api.openPath(status!.installedPath!)} title={status.installedPath}>
            Open folder
          </button>
          <span class="sep">·</span>
        {/if}
        {#if status?.lastCheck}
          <span>checked {new Date(status.lastCheck).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
          <span class="sep">·</span>
        {/if}
        <button class="link" onclick={refresh} disabled={checking || running}>
          {checking ? "checking…" : "check now"}
        </button>
      </div>
    </div>

    <div class="actions">
      {#if running}
        <button class="danger" onclick={cancel}>Cancel build</button>
      {:else}
        {#if installed}
          <button onclick={launch}>Launch</button>
        {/if}
        <button class="primary" onclick={start} disabled={!latest && !installed}>
          {primaryLabel}
        </button>
      {/if}
    </div>
  </div>

  {#if actionError}
    <div class="error mono">{actionError}</div>
  {/if}
</section>

<style>
  .hero {
    padding: 18px 20px;
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 18px;
  }

  .version-line {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-top: 2px;
  }

  .version {
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
    line-height: 1.25;
  }

  .status {
    font-size: 12.5px;
    font-weight: 600;
  }

  .status.ok {
    color: var(--ok);
  }

  .status.update {
    color: var(--accent);
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    margin-top: 3px;
  }

  .sep {
    opacity: 0.5;
  }

  .link {
    background: none;
    border: none;
    padding: 0;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-dim);
    border-radius: 0;
  }

  .link:hover:not(:disabled) {
    color: var(--accent);
    background: none;
  }

  .actions {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-shrink: 0;
  }

  .error {
    margin-top: 12px;
    padding: 9px 12px;
    border-radius: var(--radius-sm);
    background: var(--err-bg);
    color: var(--err);
    user-select: text;
  }
</style>
