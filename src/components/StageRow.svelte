<script lang="ts">
  import type { StageInfo } from "../types";

  let { stage }: { stage: StageInfo } = $props();

  const icon = $derived.by(() => {
    switch (stage.status) {
      case "done":
        return "✓";
      case "failed":
        return "✕";
      case "skipped":
        return "↷";
      default:
        return "";
    }
  });
</script>

<div class="row" class:dimmed={stage.status === "pending" || stage.status === "skipped"}>
  <div class="status-icon" data-status={stage.status}>
    {#if stage.status === "running"}
      <span class="spinner"></span>
    {:else}
      {icon}
    {/if}
  </div>
  <div class="body">
    <div class="top">
      <span class="name">{stage.name}</span>
      <span class="detail dim mono">{stage.detail}</span>
    </div>
    {#if stage.status === "running"}
      <div class="bar">
        {#if stage.progress !== null}
          <div class="fill" style={`width: ${Math.round(stage.progress * 100)}%`}></div>
        {:else}
          <div class="fill indeterminate"></div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .row {
    display: flex;
    gap: 11px;
    padding: 6px 0;
    align-items: flex-start;
  }

  .dimmed {
    opacity: 0.4;
  }

  .status-icon {
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    font-size: 12px;
    font-weight: 700;
    color: var(--text-faint);
    margin-top: 2px;
  }

  .status-icon[data-status="done"] {
    color: var(--ok);
  }

  .status-icon[data-status="failed"] {
    color: var(--err);
  }

  .spinner {
    width: 11px;
    height: 11px;
    border: 2px solid var(--accent);
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .body {
    flex: 1;
    min-width: 0;
  }

  .top {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
  }

  .name {
    font-weight: 500;
  }

  .detail {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .bar {
    height: 4px;
    background: var(--surface-2);
    border-radius: 2px;
    margin-top: 6px;
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 2px;
    transition: width 0.25s ease;
  }

  .fill.indeterminate {
    width: 30%;
    animation: slide 1.2s ease-in-out infinite;
  }

  @keyframes slide {
    0% {
      margin-left: -30%;
    }
    100% {
      margin-left: 100%;
    }
  }
</style>
