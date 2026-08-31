<script lang="ts">
  import { api } from "../api";
  import type { PipelineState } from "../types";
  import StageRow from "./StageRow.svelte";
  import LogConsole from "./LogConsole.svelte";

  let { pipeline, logLines }: { pipeline: PipelineState | null; logLines: string[] } = $props();

  let showLog = $state(false);
  let retryError = $state<string | null>(null);

  const visible = $derived(
    !!pipeline && (pipeline.running || !!pipeline.error || !!pipeline.summary)
  );

  async function retry() {
    retryError = null;
    try {
      await api.retryPipeline();
    } catch (e) {
      retryError = String(e);
    }
  }

  function fmtBytes(n: number): string {
    if (n >= 1 << 30) return (n / (1 << 30)).toFixed(2) + " GiB";
    if (n >= 1 << 20) return (n / (1 << 20)).toFixed(1) + " MiB";
    return (n / 1024).toFixed(0) + " KiB";
  }

  function fmtDuration(s: number): string {
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return m > 0 ? `${m} min ${sec} s` : `${sec} s`;
  }
</script>

{#if visible && pipeline}
  <section class="card build">
    <div class="head">
      <span class="section-label">
        {#if pipeline.running}Building{:else if pipeline.error}Build failed{:else}Build complete{/if}
      </span>
      <button class="ghost log-btn" onclick={() => (showLog = !showLog)}>
        {showLog ? "Hide log" : "Show log"}
      </button>
    </div>

    <div class="stages">
      {#each pipeline.stages as stage (stage.id)}
        <StageRow {stage} />
      {/each}
    </div>

    {#if pipeline.error}
      <div class="error-box">
        <div class="error-title">
          {pipeline.error === "Cancelled" ? "Cancelled" : `Failed${pipeline.failedStage ? ` during “${pipeline.failedStage}”` : ""}`}
        </div>
        {#if pipeline.error !== "Cancelled"}
          <div class="mono error-msg">{pipeline.error}</div>
        {/if}
        <div class="error-actions">
          <button class="primary" onclick={retry}>
            {pipeline.error === "Cancelled" ? "Resume" : "Retry from failed stage"}
          </button>
          {#if retryError}<span class="mono err-inline">{retryError}</span>{/if}
        </div>
      </div>
    {/if}

    {#if pipeline.summary}
      <div class="summary">
        <span class="check">✓</span>
        Aseprite {pipeline.summary.version} installed in {fmtDuration(pipeline.summary.elapsedSecs)}
        · {fmtBytes(pipeline.summary.installedBytes)}
        {#if pipeline.summary.cleanedBytes > 0}
          · {fmtBytes(pipeline.summary.cleanedBytes)} cleaned up
        {/if}
      </div>
    {/if}

    {#if showLog}
      <LogConsole lines={logLines} />
    {/if}
  </section>
{/if}

<style>
  .build {
    padding: 16px 20px 18px;
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
  }

  .log-btn {
    font-size: 12px;
    padding: 4px 10px;
  }

  .stages {
    display: flex;
    flex-direction: column;
  }

  .error-box {
    background: var(--err-bg);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    margin-top: 12px;
  }

  .error-title {
    font-weight: 600;
    color: var(--err);
    margin-bottom: 5px;
  }

  .error-msg {
    color: var(--text);
    white-space: pre-wrap;
    user-select: text;
    max-height: 120px;
    overflow-y: auto;
  }

  .error-actions {
    margin-top: 10px;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .err-inline {
    color: var(--err);
  }

  .summary {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 12px;
    font-weight: 500;
  }

  .check {
    color: var(--ok);
    font-weight: 700;
    flex-shrink: 0;
  }
</style>
