export function formatTimestamp(ts: string): string {
  if (!ts) return "";
  try {
    const d = new Date(ts);
    if (isNaN(d.getTime())) return ts;
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    const h = String(d.getHours()).padStart(2, "0");
    const min = String(d.getMinutes()).padStart(2, "0");
    return `${y}-${m}-${day} ${h}:${min}`;
  } catch {
    return ts;
  }
}

export function formatRelativeTime(ts: string): string {
  if (!ts) return "";
  try {
    const now = Date.now();
    const then = new Date(ts).getTime();
    if (isNaN(then)) return ts;
    const diff = now - then;
    const seconds = Math.floor(diff / 1000);
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);

    if (seconds < 60) return "刚刚";
    if (minutes < 60) return `${minutes}分钟前`;
    if (hours < 24) return `${hours}小时前`;
    if (days < 30) return `${days}天前`;
    return formatTimestamp(ts);
  } catch {
    return ts;
  }
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "-";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/**
 * 完整金额格式：$1,234.56（千分位 + 2 位小数）。
 * 用于悬浮提示与非拥挤区域；非有限数值按 0 处理。
 */
export function formatCost(n: number): string {
  const value = Number.isFinite(n) ? n : 0;
  const sign = value < 0 ? "-" : "";
  return (
    sign +
    "$" +
    Math.abs(value).toLocaleString("en-US", {
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    })
  );
}

/**
 * 紧凑金额格式：KPI 卡片主值专用，保证永不溢出。
 * 分档边界按四舍五入后的显示值调整，避免出现 $1000.0 这类跨档回跳。
 * 完整金额请配合 formatCost 以 title 悬浮提示展示。
 */
export function formatCostCompact(n: number): string {
  const value = Number.isFinite(n) ? n : 0;
  const sign = value < 0 ? "-" : "";
  const abs = Math.abs(value);
  if (abs >= 999_995) return `${sign}$${(abs / 1_000_000).toFixed(2)}M`;
  if (abs >= 999.95) return `${sign}$${(abs / 1_000).toFixed(2)}K`;
  if (abs >= 99.995) return `${sign}$${abs.toFixed(1)}`;
  return `${sign}$${abs.toFixed(2)}`;
}

/** token 计数紧凑格式：380 → "380"，1200 → "1.2k"，2_500_000 → "2.5M" */
export function formatTokenCount(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "0";
  if (n < 1000) return `${Math.round(n)}`;
  const trim = (v: number) => v.toFixed(1).replace(/\.0$/, "");
  if (n < 1_000_000) return `${trim(n / 1000)}k`;
  return `${trim(n / 1_000_000)}M`;
}

/** Replace home directory prefix with ~ for cleaner display and privacy. */
export function shortenHomePath(path: string): string {
  if (!path) return "";
  const normalized = path.replace(/\\/g, "/");
  const homePatterns = [/\/Users\/[^/\s]+/g, /\/home\/[^/\s]+/g];
  const windowsHomePattern = /[A-Z]:\/Users\/[^/\s]+/gi;
  let shortened = normalized.replace(windowsHomePattern, "~");
  for (const pat of homePatterns) {
    shortened = shortened.replace(pat, "~");
  }
  return shortened;
}

