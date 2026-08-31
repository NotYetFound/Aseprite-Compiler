<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../api";
  import type { ToolReport } from "../types";

  let report = $state<ToolReport | null>(null);
  let loading = $state(false);
  let provisioning = $state(false);
  let error = $state<string | null>(null);
  let copied = $state(false);

  const provisionable = $derived(
    report?.tools.some((t) => !t.ok && t.provisionable) ?? false
  );

  async function check() {
    loading = true;
    error = null;
    try {
      report = await api.checkTools();
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function provision() {
    provisioning = true;
    error = null;
    try {
      await api.provisionTools();
      report = await api.checkTools();
    } catch (e) {
      error = String(e);
    } finally {
      provisioning = false;
    }
  }

  async function copyHelper() {
    if (!report?.helperCommand) return;
    await api.copyToClipboard(report.helperCommand);
    copied = true;
    setTimeout(() => (copied = false), 1800);
  }

  onMount(check);
</script>

<section class="card tools">
  <div class="head">
    <span class="section-label">Build tools</span>
    <div class="head-right">
      {#if report?.allOk}
        <span class="all-ok">✓ all ready</span>
      {/if}
      <button class="ghost small" onclick={check} disabled={loading}>
        {loading ? "Checking…" : "Re-check"}
      </button>
    </div>
  </div>

  {#if report}
    <div class="chips">
      {#each report.tools as tool (tool.id)}
        <span class="chip" class:bad={!tool.ok} title={tool.detail}>
          <span class="dot" class:ok={tool.ok} class:err={!tool.ok}></span>
          {tool.name}
        </span>
      {/each}
    </div>

    {#if provisionable}
      <div class="action">
        <div class="action-text">
          <div class="action-title">Portable tools missing</div>
          <div class="dim small">Downloaded into the app's own folder — nothing touches your system.</div>
        </div>
        <button class="primary" onclick={provision} disabled={provisioning}>
          {provisioning ? "Installing…" : "Install portable tools"}
        </button>
      </div>
    {/if}

    {#if report.helperCommand}
      <div class="helper">
        <div class="dim small">{report.helperLabel ?? "Install the missing system packages:"}</div>
        <div class="cmd-row">
          <code class="mono cmd">{report.helperCommand}</code>
          <button class="small" onclick={copyHelper}>{copied ? "Copied ✓" : "Copy"}</button>
        </div>
      </div>
    {/if}
  {/if}

  {#if error}
    <div class="mono error">{error}</div>
  {/if}
</section>

<style>
  .tools {
    padding: 16px 20px;
  }

  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 11px;
  }

  .head-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .all-ok {
    font-size: 12px;
    font-weight: 600;
    color: var(--ok);
  }

  .small {
    font-size: 12px;
    padding: 4px 10px;
  }

  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    font-weight: 500;
    padding: 4px 10px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
    cursor: default;
  }

  .chip.bad {
    background: var(--err-bg);
    color: var(--err);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
  }

  .dot.ok {
    background: var(--ok);
  }

  .dot.err {
    background: var(--err);
  }

  .action {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 14px;
    margin-top: 14px;
    padding: 12px 14px;
    border-radius: var(--radius-sm);
    background: var(--surface-2);
  }

  .action-title {
    font-weight: 600;
  }

  .small {
    font-size: 12px;
  }

  .helper {
    margin-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 7px;
  }

  .cmd-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }

  .cmd {
    flex: 1;
    background: var(--surface-2);
    border-radius: var(--radius-sm);
    padding: 8px 12px;
    overflow-x: auto;
    white-space: nowrap;
    user-select: text;
  }

  .error {
    margin-top: 10px;
    color: var(--err);
    user-select: text;
  }
</style>
