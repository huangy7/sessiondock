// 快捷用语的日期变量展开：{{今天}} 等占位符在填入输入框时展开为本地日期（yyyy-mm-dd）
// 日期计算复用 week-range.ts 的本地时区口径（避免 UTC 偏移把凌晨算成昨天）

import { formatLocalDate, currentWeekRange } from "./week-range";

function addDays(base: Date, days: number): Date {
  const d = new Date(base);
  d.setDate(d.getDate() + days);
  return d;
}

/** 变量表：key 为 {{}} 内的名字。新增变量只需在这里加一行 + 一条测试 */
function resolveVariable(name: string, now: Date): string | null {
  const week = currentWeekRange(now);
  const monday = new Date(week.start + "T00:00:00");
  switch (name) {
    case "今天":
      return formatLocalDate(now);
    case "昨天":
      return formatLocalDate(addDays(now, -1));
    case "本周一":
      return week.start;
    case "本周五":
      return week.end;
    case "上周一":
      return formatLocalDate(addDays(monday, -7));
    case "上周五":
      return formatLocalDate(addDays(monday, -3));
    default:
      return null;
  }
}

/** 展开文本中的所有 {{变量}}；未知变量原样保留（用户可见、可手改） */
export function expandPhraseVariables(text: string, now: Date = new Date()): string {
  return text.replace(/\{\{([^{}]+)\}\}/g, (raw, name: string) => {
    return resolveVariable(name.trim(), now) ?? raw;
  });
}
