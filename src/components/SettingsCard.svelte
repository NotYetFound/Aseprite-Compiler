<script lang="ts">
  import type { Settings } from "../types";
  import { api } from "../api";

  let { settings, onSave }: { settings: Settings; onSave: (s: Settings) => Promise<void> } = $props();

  // Local editable copy; saved on every change.
  let s = $state<Settings>({ ...settings });
  let confirmUninstall = $state(false);
  let uninstallError = $state<string | null>(null);
  let saveError = $state<string | null>(null);

  // Emptying a number input yields null, and out-of-range values are typable
  // despite min/max — normalize before the backend sees them.
  function sanitized(): Settings {
    const int = (v: unknown, fallback: number, lo: number, hi: number) => {
      const n = Math.floor(Number(v));
      return Number.isFinite(n) ? Math.min(hi, Math.max(lo, n)) : fallback;
    };
    return {
      ...s,
      parallelJobs: int(s.parallelJobs, 0, 0, 128),
      watcherIntervalHours: int(s.watcherIntervalHours, 12, 1, 168),
    };
  }

  async function save() {
    saveError = null;
    const next = sanitized();
    try {
      await onSave(next);
      s = { ...next };
    } catch (e) {
      saveError = String(e);
      try {
        s = await api.getSettings(); // revert to what's actually persisted
      } catch {
        // keep the local values if even reading back fails
      }
    }
  }

  async function uninstall() {
    uninstallError = null;
    try {
      await api.uninstallAseprite();
      confirmUninstall = false;
    } catch (e) {
      uninstallError = String(e);
    }
  }
</script>

<section class="card settings">
  <div class="group-label section-label">Build</div>
  <div class="rows">
    <label class="row">
      <div class="text">
        <div class="label">Release channel</div>
        <div class="dim small">Stable is recommended; Beta follows pre-releases.</div>
      </div>
      <select bind:value={s.channel} onchange={save}>
        <option value="stable">Stable</option>
        <option value="beta">Beta</option>
      </select>
    </label>

    <label class="row">
      <div class="text">
        <div class="label">Install location</div>
        <div class="dim small">Empty = default per-user location.</div>
      </div>
      <input class="text-input" type="text" bind:value={s.installDir} onchange={save} placeholder="default" />
    </label>

    <label class="row">
      <div class="text">
        <div class="label">Parallel compile jobs</div>
        <div class="dim small">0 = all CPU cores.</div>
      </div>
      <input class="num" type="number" min="0" max="128" bind:value={s.parallelJobs} onchange={save} />
    </label>

    <label class="row">
      <div class="text">
        <div class="label">Clean up after build</div>
        <div class="dim small">Delete source and build files after installing — saves several GB.</div>
      </div>
      <input type="checkbox" bind:checked={s.cleanupAfterBuild} onchange={save} />
    </label>
  </div>

  <div class="group-label section-label spaced">Updates</div>
  <div class="rows">
    <label class="row">
      <div class="text">
        <div class="label">Watch for new releases</div>
        <div class="dim small">Notify shows a notification; Auto also starts the build.</div>
      </div>
      <select bind:value={s.watcherMode} onchange={save}>
        <option value="off">Off</option>
        <option value="notify">Notify</option>
        <option value="auto">Auto-build</option>
      </select>
    </label>

    <label class="row">
      <div class="text">
        <div class="label">Check every</div>
        <div class="dim small">Hours between release checks.</div>
      </div>
      <input class="num" type="number" min="1" max="168" bind:value={s.watcherIntervalHours} onchange={save} />
    </label>

    <label class="row">
      <div class="text">
        <div class="label">System tray icon</div>
        <div class="dim small">Keep watching for updates when the window is closed.</div>
      </div>
      <input type="checkbox" bind:checked={s.trayEnabled} onchange={save} />
    </label>

    <label class="row">
      <div class="text">
        <div class="label">Start in tray at login</div>
        <div class="dim small">Launch minimized when you log in (needs the tray icon).</div>
      </div>
      <input type="checkbox" bind:checked={s.startMinimized} onchange={save} disabled={!s.trayEnabled} />
    </label>
  </div>

  <div class="rows uninstall">
    <div class="row">
      <div class="text">
        <div class="label">Uninstall Aseprite</div>
        <div class="dim small">Removes the build and launcher entry; your Aseprite files stay.</div>
      </div>
      {#if confirmUninstall}
        <span class="confirm">
          <button class="danger" onclick={uninstall}>Confirm</button>
          <button onclick={() => (confirmUninstall = false)}>Keep</button>
        </span>
      {:else}
        <button class="danger" onclick={() => (confirmUninstall = true)}>Uninstall…</button>
      {/if}
    </div>
    {#if uninstallError}
      <div class="mono err">{uninstallError}</div>
    {/if}
  </div>

  {#if saveError}
    <div class="mono err">Could not save settings: {saveError}</div>
  {/if}
</section>

<style>
  .settings {
    padding: 16px 20px 10px;
  }

  .group-label.spaced {
    margin-top: 18px;
  }

  .rows {
    display: flex;
    flex-direction: column;
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 22px;
    padding: 11px 0;
  }

  .row + .row {
    border-top: 1px solid var(--divider);
  }

  .uninstall {
    margin-top: 14px;
    border-top: 1px solid var(--divider);
  }

  .text {
    min-width: 0;
  }

  .label {
    font-weight: 550;
  }

  .small {
    font-size: 12px;
    line-height: 1.4;
  }

  .num {
    width: 64px;
  }

  .text-input {
    width: 180px;
  }

  .confirm {
    display: flex;
    gap: 8px;
  }

  .err {
    color: var(--err);
    padding: 4px 0 10px;
    user-select: text;
  }
</style>
