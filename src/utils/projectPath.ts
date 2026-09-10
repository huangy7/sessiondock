function normalizePath(path: string): string {
  return path.replace(/[\\/]+$/, "");
}

export function isSubPath(filePath: string, projectPath: string): boolean {
  const root = normalizePath(projectPath);
  return filePath === root || filePath.startsWith(root + "/") || filePath.startsWith(root + "\\");
}

export function findBestProjectPath(filePath: string, projectPaths: readonly string[]): string | null {
  let best: string | null = null;
  for (const projectPath of projectPaths) {
    if (!isSubPath(filePath, projectPath)) continue;
    if (!best || normalizePath(projectPath).length > normalizePath(best).length) {
      best = projectPath;
    }
  }
  return best;
}

export function basename(path: string): string {
  if (!path) return "";
  const parts = path.replace(/\\/g, "/").split("/").filter(Boolean);
  return parts.pop() || path;
}
