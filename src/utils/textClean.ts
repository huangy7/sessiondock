/**
 * Clean system-injected XML tags, skill command wrappers, and image placeholders from user message text for clean UI rendering.
 */
export function cleanUserText(text: string): string {
  if (!text) return "";

  let result = text;

  // <cb_summary> 压缩检查点不展示（WorkBuddy 上下文压缩行）
  if (result.trimStart().startsWith("<cb_summary")) return "";

  // Extract <command-args> if present
  const commandArgsMatch = result.match(/<command-args>([\s\S]*?)<\/command-args>/);
  if (commandArgsMatch) {
    result = commandArgsMatch[1];
  }

  // Extract <user_query> if present（取最后一个：WorkBuddy 会把上下文包在前面的同名标签里）
  const userQueryMatches = [...result.matchAll(/<user_query>\s*([\s\S]*?)\s*<\/user_query>/g)];
  if (userQueryMatches.length > 0) {
    result = userQueryMatches[userQueryMatches.length - 1][1];
  }

  result = result
    .replace(/<command-message>[\s\S]*?<\/command-message>\s*/g, "")
    .replace(/<command-name>[\s\S]*?<\/command-name>\s*/g, "")
    .replace(/<system-reminder[^>]*>[\s\S]*?<\/system-reminder>\s*/g, "")
    .replace(/<user_info>[\s\S]*?<\/user_info>\s*/g, "")
    .replace(/<git_status>[\s\S]*?<\/git_status>\s*/g, "")
    .replace(/<attached_files>[\s\S]*?<\/attached_files>\s*/g, "")
    .replace(/<agent_transcripts>[\s\S]*?<\/agent_transcripts>\s*/g, "")
    .replace(/<agent_skills>[\s\S]*?<\/agent_skills>\s*/g, "")
    .replace(/<rules>[\s\S]*?<\/rules>\s*/g, "")
    .replace(/\[Image(?:\s*#\d+)?\]\s*/g, "");

  return result.trim();
}
