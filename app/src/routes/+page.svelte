<script lang="ts">
  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';

  onMount(() => {
    void goto('/launcher', { replaceState: true });
  });
</script>

<p style="padding:16px">正在打开 miniBot…</p>
