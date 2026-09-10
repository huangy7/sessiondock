import { computed } from "vue";
import { useBlockedFolders } from "./useBlockedFolders";

/**
 * 项目过滤 Composable（已与全局 useBlockedFolders 屏蔽文件夹统一数据源）。
 * 保留此接口以向下兼容各调用处与单测，统一持久化至后端 SQLite blocked_folders 表。
 */
export function useProjectFilter() {
  const { blockedFolders, blockFolder, unblockFolder, isBlocked: isFolderBlocked } = useBlockedFolders();

  const blockedProjects = computed(() => new Set(blockedFolders.value));
  const hiddenProjects = computed(() => new Set(blockedFolders.value));

  function blockProject(path: string) {
    void blockFolder(path);
  }

  function unblockProject(path: string) {
    void unblockFolder(path);
  }

  function isBlocked(path: string): boolean {
    return isFolderBlocked(path);
  }

  function hideProject(path: string) {
    void blockFolder(path);
  }

  function unhideProject(path: string) {
    void unblockFolder(path);
  }

  function isHidden(path: string): boolean {
    return isFolderBlocked(path);
  }

  return {
    blockedProjects,
    hiddenProjects,
    blockProject,
    unblockProject,
    isBlocked,
    hideProject,
    unhideProject,
    isHidden,
  };
}
