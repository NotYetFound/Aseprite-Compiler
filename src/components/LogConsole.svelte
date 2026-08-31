<script lang="ts">
  let { lines }: { lines: string[] } = $props();

  let box: HTMLDivElement | undefined = $state();
  let autoScroll = $state(true);

  $effect(() => {
    // touch lines so the effect re-runs on new output
    void lines.length;
    if (autoScroll && box) box.scrollTop = box.scrollHeight;
  });

  function onScroll() {
    if (!box) return;
    autoScroll = box.scrollHeight - box.scrollTop - box.clientHeight < 40;
  }
</script>

<div class="console mono" bind:this={box} onscroll={onScroll}>
  {#if lines.length === 0}
    <div class="dim">No output yet.</div>
  {:else}
    {#each lines as line}
      <div class="line">{line}</div>
    {/each}
  {/if}
</div>

<style>
  .console {
    background: var(--surface-2);
    border-radius: var(--radius-sm);
    height: 240px;
    margin-top: 12px;
    overflow-y: auto;
    padding: 10px 12px;
    user-select: text;
  }

  .line {
    white-space: pre-wrap;
    word-break: break-all;
    line-height: 1.5;
  }
</style>
