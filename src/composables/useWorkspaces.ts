import { ref } from "vue";
import type { ProjectInfo } from "../types/session";
import { useProjectFilter } from "./useProjectFilter";

const STORAGE_KEY = "claudia-manual-workspaces";

const manualPaths = ref<string[]>(loadFromStorage());

function loadFromStorage(): string[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveToStorage() {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(manualPaths.value));
}

export function useWorkspaces() {
  const { isBlocked, isHidden } = useProjectFilter();

  function addWorkspace(path: string) {
    if (!manualPaths.value.includes(path)) {
      manualPaths.value.push(path);
      saveToStorage();
    }
  }

  function removeWorkspace(path: string) {
    manualPaths.value = manualPaths.value.filter((p) => p !== path);
    saveToStorage();
  }

  function mergedProjectPaths(projects: ProjectInfo[]): string[] {
    const seen = new Set<string>();
    const result: string[] = [];
    for (const p of projects) {
      if (!seen.has(p.original_path) && !isBlocked(p.original_path) && !isHidden(p.original_path)) {
        seen.add(p.original_path);
        result.push(p.original_path);
      }
    }
    for (const p of manualPaths.value) {
      if (!seen.has(p) && !isBlocked(p) && !isHidden(p)) {
        seen.add(p);
        result.push(p);
      }
    }
    return result;
  }

  return { manualPaths, addWorkspace, removeWorkspace, mergedProjectPaths };
}
