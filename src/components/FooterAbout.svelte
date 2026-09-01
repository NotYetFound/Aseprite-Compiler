<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "../api";

  let version = $state("");
  let updateState = $state<"idle" | "checking" | "none" | "available" | "installing" | "error">("idle");
  let updateVersion = $state<string | null>(null);
  let updateError = $state<string | null>(null);

  onMount(async () => {
    try {
      version = await api.getAppVersion();
    } catch {
      // backend unavailable
    }
  });

  async function checkUpdate() {
    updateState = "checking";
    updateError = null;
    try {
      updateVersion = await api.checkAppUpdate();
      updateState = updateVersion ? "available" : "none";
    } catch (e) {
      updateState = "error";
      updateError = `update check failed: ${e}`;
    }
  }

  async function installUpdate() {
    updateState = "installing";
    updateError = null;
    try {
      await api.installAppUpdate(); // restarts on success
    } catch (e) {
      updateState = "error";
      updateError = `update install failed: ${e}`;
    }
  }

  let diagState = $state<"idle" | "working" | "done" | "error">("idle");
  let diagPath = $state("");
  let diagError = $state<string | null>(null);

  async function exportDiagnostics() {
    diagState = "working";
    diagError = null;
    try {
      diagPath = await api.exportDiagnostics();
      diagState = "done";
    } catch (e) {
      diagState = "error";
      diagError = `diagnostics export failed: ${e}`;
    }
  }
</script>

<footer>
  <div class="line">
    <span>Aseprite Compiler{version ? ` v${version}` : ""}</span>
    <span class="sep">·</span>
    {#if updateState === "available"}
      <button class="link accent" onclick={installUpdate}>Install v{updateVersion} &amp; restart</button>
    {:else if updateState === "installing"}
      <span>installing update…</span>
    {:else if updateState === "checking"}
      <span>checking…</span>
    {:else if updateState === "none"}
      <span>app is up to date</span>
    {:else}
      <button class="link" onclick={checkUpdate}>check for app updates</button>
    {/if}
    <span class="sep">·</span>
    {#if diagState === "working"}
      <span>exporting…</span>
    {:else if diagState === "done"}
      <button class="link" onclick={() => api.revealPath(diagPath)}>show diagnostics zip</button>
    {:else}
      <button class="link" onclick={exportDiagnostics}>export diagnostics</button>
    {/if}
    <span class="sep">·</span>
    <button class="link" onclick={() => api.openPath("https://github.com/aseprite/aseprite/blob/main/EULA.txt")}>EULA</button>
    <span class="sep">·</span>
    <button class="link" onclick={() => api.openPath("https://www.aseprite.org/")}>aseprite.org</button>
  </div>
  {#if updateState === "error" && updateError}
    <div class="err">{updateError}</div>
  {/if}
  {#if diagState === "error" && diagError}
    <div class="err">{diagError}</div>
  {/if}
  <div class="notice">
    Not affiliated with Igara Studio. Aseprite is compiled locally from the official
    source — no binaries are distributed. If you enjoy it, consider buying Aseprite.
  </div>
</footer>

<style>
  footer {
    padding: 10px 6px 0;
    display: flex;
    flex-direction: column;
    gap: 5px;
    font-size: 12px;
    color: var(--text-faint);
  }

  .line {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
  }

  .sep {
    opacity: 0.55;
  }

  .link {
    background: none;
    border: none;
    padding: 0;
    font-size: 12px;
    font-weight: 500;
    color: var(--text-dim);
    text-decoration: underline;
    text-underline-offset: 2.5px;
    text-decoration-color: color-mix(in srgb, currentColor 40%, transparent);
  }

  .link:hover {
    color: var(--text);
    background: none;
    border: none;
  }

  .link.accent {
    color: var(--accent);
    font-weight: 600;
  }

  .err {
    color: var(--err);
  }

  .notice {
    line-height: 1.5;
    max-width: 520px;
  }
</style>
