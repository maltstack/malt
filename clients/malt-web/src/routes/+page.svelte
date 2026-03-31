<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { listSessions, createSession, getOutput, sendInput, type StyledSpan } from '$lib/api';

  let sessionId: number | null = null;
  let rows: StyledSpan[][] = [];
  let inputBuffer = '';
  let pollInterval: ReturnType<typeof setInterval> | null = null;
  let connected = false;
  let error = '';

  onMount(async () => {
    try {
      // Try to connect to daemon
      const sessions = await listSessions();
      if (sessions.length > 0) {
        sessionId = sessions[0].id;
      } else {
        const session = await createSession('web');
        sessionId = session.id;
      }
      connected = true;

      // Poll for output
      pollInterval = setInterval(async () => {
        if (sessionId) {
          try {
            rows = await getOutput(sessionId);
          } catch (e) {
            // Connection lost
          }
        }
      }, 100);
    } catch (e) {
      error = 'Cannot connect to MALT daemon. Run: malt daemon';
    }
  });

  onDestroy(() => {
    if (pollInterval) clearInterval(pollInterval);
  });

  function handleKeydown(event: KeyboardEvent) {
    if (!sessionId) return;

    let input = '';
    if (event.key === 'Enter') {
      input = inputBuffer + '\r';
      inputBuffer = '';
    } else if (event.key === 'Backspace') {
      inputBuffer = inputBuffer.slice(0, -1);
      return;
    } else if (event.key.length === 1 && !event.ctrlKey && !event.altKey) {
      inputBuffer += event.key;
      return;
    } else if (event.ctrlKey && event.key === 'c') {
      input = '\x03'; // Ctrl+C
    } else {
      return;
    }

    event.preventDefault();
    if (input) {
      sendInput(sessionId, input);
    }
  }

  function rgbToColor(rgb: [number, number, number]): string {
    return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="terminal">
  {#if error}
    <div class="error">{error}</div>
  {:else if !connected}
    <div class="connecting">Connecting to MALT daemon...</div>
  {:else}
    <div class="output">
      {#each rows as row, y}
        <div class="row">
          {#each row as span}
            <span
              style="color: {rgbToColor(span.fg)}; background: {rgbToColor(span.bg)}; {span.b ? 'font-weight: bold;' : ''}"
            >{span.t}</span>
          {/each}
        </div>
      {/each}
    </div>
    <div class="input-line">
      <span class="prompt">&gt; </span>
      <span class="input-text">{inputBuffer}</span>
      <span class="cursor">&#x2588;</span>
    </div>
  {/if}
</div>

<style>
  .terminal {
    padding: 8px 12px;
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .output {
    flex: 1;
    overflow-y: auto;
    white-space: pre;
    font-family: var(--font);
    line-height: 1.4;
  }
  .row {
    min-height: 1.4em;
  }
  .input-line {
    padding: 4px 0;
    border-top: 1px solid #333;
    display: flex;
    align-items: center;
  }
  .prompt { color: var(--accent); }
  .input-text { color: var(--fg); }
  .cursor {
    color: var(--accent);
    animation: blink 1s step-end infinite;
  }
  @keyframes blink {
    50% { opacity: 0; }
  }
  .error { color: #ff5555; padding: 20px; }
  .connecting { color: #888; padding: 20px; }
</style>
