<script setup lang="ts">
import ArchiveRetentionSettings from "./ArchiveRetentionSettings.vue";
import CacheMaintenanceSettings from "./CacheMaintenanceSettings.vue";
import type { CliId } from "../types/cli";

defineProps<{ initialCliId?: CliId }>();
</script>

<template>
  <div class="data-session-settings">
    <ArchiveRetentionSettings />
    <div class="card-divider" />
    <CacheMaintenanceSettings />
  </div>
</template>

<style scoped>
.data-session-settings {
  display: flex;
  flex-direction: column;
}
.card-divider {
  border-top: 1px solid var(--color-border);
}
</style>
