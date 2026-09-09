<script lang="ts">
  import { onMount } from "svelte";
  import {
    applyPalette,
    getPalette,
    listConnections,
    ping,
    type ConnectionConfig,
  } from "./lib/tauri";
  import Sidebar from "./lib/components/Sidebar.svelte";
  import TopBar from "./lib/components/TopBar.svelte";

  let connections = $state<ConnectionConfig[]>([]);
  let selectedId = $state<number | null>(null);
  let backendStatus = $state("Starting…");
  let error = $state<string | null>(null);
  let showNewConnection = $state(false);

  onMount(async () => {
    try {
      const [pong, palette, conns] = await Promise.all([
        ping("hello"),
        getPalette(),
        listConnections(),
      ]);
      applyPalette(palette);
      connections = conns;
      backendStatus = `Backend ready · ping ${pong.latency_ms}ms`;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      backendStatus = "Backend unreachable";
    }
  });

  function handleSelect(id: number): void {
    // Milestone 2: invoke connect + list_tables; light up the dot for now.
    selectedId = id;
  }
</script>

<TopBar {backendStatus} onNewConnection={() => (showNewConnection = true)} />

<div class="layout">
  <Sidebar {connections} {selectedId} onSelect={handleSelect} />

  <main class="main">
    {#if error}
      <section class="card">
        <h2>ERROR</h2>
        <p class="error">{error}</p>
      </section>
    {/if}

    <section class="card">
      <h2>TABLE</h2>
      <!-- Milestone 2: virtualized DataGrid + FilterBuilder + pagination. -->
      <p class="muted">Pick a connection in the sidebar to browse tables.</p>
    </section>

    <section class="card">
      <h2>QUERY EDITOR</h2>
      <!-- Milestone 3: CodeMirror SQL editor + history + results + export. -->
      <p class="muted">Run ad-hoc SQL here once a connection is live.</p>
    </section>

    {#if showNewConnection}
      <section class="card">
        <h2>NEW CONNECTION</h2>
        <!-- Milestone 2: connection form (engine/host/port/user/password). -->
        <p class="muted">Connection form lands in Milestone 2.</p>
        <button class="btn-ghost" onclick={() => (showNewConnection = false)}>
          Close
        </button>
      </section>
    {/if}
  </main>
</div>
