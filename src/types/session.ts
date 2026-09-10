import type { CliId } from "./cli";

export interface SessionIdentity {
  cliId: CliId;
  filePath: string;
}

export function sessionIdentityKey(value: SessionIdentity): string {
  return `${value.cliId}\u0000${value.filePath}`;
}

export interface ProjectInfo {
  encoded_dir: string;
  original_path: string;
  sessions: SessionInfo[];
}

export interface AggregatedProjectInfo extends ProjectInfo {
  /** Stable normalized path used to merge the same workspace across CLIs. */
  project_key: string;
  /** CLI sources represented by the currently visible sessions. */
  cli_ids: CliId[];
}

export interface SessionInfo {
  session_id: string;
  file_path: string;
  display_name: string;
  timestamp: string;
  file_size: number;
  git_branch: string;
  has_archive_snapshot: boolean;
  is_archived: boolean;
  cli_id: string;
}

export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens: number;
  cache_read_input_tokens: number;
}

export interface ChatMessage {
  role: string;
  timestamp: string;
  model: string | null;
  token_usage?: TokenUsage | null;
  content_parts: ContentPart[];
  is_meta?: boolean;
  /** Transcript entry uuid (Claude only) — stable anchor for fork-from-here. */
  uuid?: string;
}

export interface SubagentInfo {
  file_path: string;
  label: string;
}

export type ContentPart =
  | { type: "text"; text: string }
  | { type: "tool_use"; summary: string; tool_name: string; input: string; tool_use_id?: string }
  | { type: "tool_result"; summary: string; content: string; is_error: boolean; tool_use_id?: string }
  | { type: "thinking"; thinking: string }
  | { type: "image"; media_type: string; data: string }
  | { type: "image_ref"; path: string }
  | { type: "image_meta" };

export interface SearchResult {
  session_id: string;
  file_path: string;
  display_name: string;
  project_path: string;
  snippet: string;
  match_count: number;
  first_match_message_index: number | null;
  cli_id: string;
}

export interface ContextMenuItem {
  label: string;
  action?: () => void | unknown | Promise<unknown>;
  separator?: boolean;
  icon?: string;
  danger?: boolean;
  closeOnClick?: boolean;
  children?: ContextMenuItem[];
}

export interface UsageRecord {
  date: string;
  model: string;
  input_tokens: number;
  output_tokens: number;
  cache_creation_tokens: number;
  cache_read_tokens: number;
  duration_ms: number | null;
  project: string;
}

export interface BookmarkInfo {
  cliId: CliId;
  sessionId: string;
  messageIndex: number;
  note?: string;
  createdAt: string;
}

export interface SessionStats {
  total_input_tokens: number;
  total_output_tokens: number;
  total_cache_creation_tokens: number;
  total_cache_read_tokens: number;
  total_duration_ms: number;
  turn_count: number;
}

export interface PaginatedProjects {
  projects: ProjectInfo[];
  has_more: boolean;
  total_sessions: number;
}

export interface SessionLoadResult {
  messages: ChatMessage[];
  offset: number;
  subagent_map: Record<string, SubagentInfo>;
}
