<script lang="ts">
  import type { ConnectionConfig } from "../tauri";

  interface Props {
    connections: ConnectionConfig[];
    selectedId: number | null;
    onSelect: (id: number) => void;
  }

  let { connections, selectedId, onSelect }: Props = $props();
</script>

<aside class="sidebar">
  <h2>CONNECTIONS</h2>
  {#if connections.length === 0}
    <p class="muted">No saved connections yet.</p>
  {:else}
    {#each connections as conn (conn.id ?? conn.name)}
      <div
        class="conn-row"
        role="button"
        tabindex="0"
        onclick={() => conn.id != null && onSelect(conn.id)}
        onkeydown={(e) => {
          if ((e.key === "Enter" || e.key === " ") && conn.id != null) onSelect(conn.id);
        }}
      >
        <span class="dot" class:live={conn.id === selectedId}></span>
        <span>{conn.name}</span>
        <span class="muted">{conn.engine}</span>
      </div>
    {/each}
  {/if}

  <h2>DATABASES</h2>
  <!-- Milestone 2: live database/table tree (connect → list_tables). -->
  <p class="muted">Connect a server to browse databases.</p>
</aside>
