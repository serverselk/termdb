<script lang="ts">
  interface Props {
    backendStatus: string;
    onNewConnection: () => void;
  }

  let { backendStatus, onNewConnection }: Props = $props();
</script>

<header class="topbar">
  <span class="brand">TERMDB</span>
  <span class="muted">{backendStatus}</span>
  <span class="spacer"></span>
  <button class="btn-primary" onclick={onNewConnection}>+ NEW CONNECTION</button>
</header>
