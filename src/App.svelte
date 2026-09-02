<script lang="ts">
  import { onMount } from "svelte";
  import Hero from "./components/Hero.svelte";
  import BuildCard from "./components/BuildCard.svelte";
  import Tools from "./components/Tools.svelte";
  import SettingsCard from "./components/SettingsCard.svelte";
  import FooterAbout from "./components/FooterAbout.svelte";
  import { api, onPipelineLog, onPipelineState, onStatusChanged } from "./api";
  import type { PipelineState, Settings, StatusInfo } from "./types";

  let status = $state<StatusInfo | null>(null);
  let pipeline = $state<PipelineState | null>(null);
  let settings = $state<Settings | null>(null);
  let logLines = $state<string[]>([]);

  // The console shows a short tail; complete logs are kept on disk. A small
  // cap keeps DOM updates cheap when ninja emits hundreds of lines a second.
  const MAX_LOG = 500;

  onMount(() => {
    const unlisteners: Array<Promise<() => void>> = [
      onPipelineState((s) => (pipeline = s)),
      onStatusChanged((s) => (status = s)),
      onPipelineLog((line) => {
        logLines.push(line);
        if (logLines.length > MAX_LOG) logLines.splice(0, logLines.length - MAX_LOG);
      }),
    ];

    (async () => {
      try {
        settings = await api.getSettings();
        pipeline = await api.getPipelineState();
        // Merge the tail with any events that arrived while fetching it
        // (a line emitted in that window is already part of the tail).
        const pending = logLines;
        const tail = await api.getLogTail();
        logLines = tail.concat(pending.filter((l) => !tail.includes(l))).slice(-MAX_LOG);
        status = await api.getStatus(false);
      } catch {
        return; // backend unavailable
      }
      try {
        status = await api.getStatus(true);
      } catch {
        // offline is fine; keep cached status
      }
    })();

    return () => {
      for (const u of unlisteners) u.then((fn) => fn());
    };
  });

  async function saveSettings(next: Settings) {
    // Persist first; only reflect the change locally once it stuck.
    await api.setSettings(next);
    settings = next;
  }

  async function refreshStatus() {
    status = await api.getStatus(true);
  }
</script>

<main>
  <div class="column">
    <Hero {status} {pipeline} onRefresh={refreshStatus} />
    <BuildCard {pipeline} {logLines} />
    {#if settings}
      <SettingsCard {settings} onSave={saveSettings} />
    {/if}
    <Tools />
    <FooterAbout />
  </div>
</main>

<style>
  main {
    height: 100%;
    overflow-y: auto;
  }

  .column {
    max-width: 640px;
    margin: 0 auto;
    padding: 26px 22px 40px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
</style>
