import type { ChatMessage, TokenUsage } from "./session";

export interface ToolBadgeItem {
  id: string;
  tool_name: string;
  summary: string;
  input: string;
  content?: string;
  is_error: boolean;
  tool_use_id?: string;
  originalIndex: number;
  partIndex?: number;
}

export interface ToolBadgeGroup {
  tool_name: string;
  count: number;
  hasError: boolean;
  items: ToolBadgeItem[];
}

export interface ToolGroupSegment {
  type: 'tool_group';
  groups: ToolBadgeGroup[];
  hasError: boolean;
}

export interface TextSegment {
  type: 'text';
  text: string;
  originalIndex: number;
  /** 该段首个 text part 在原始消息 content_parts 内的下标（打包消息如 DSH
      [thinking, text, tool_use] 时非 0；渲染缓存按 (originalIndex, partIndex) 回查） */
  partIndex: number;
  /** 合并进该段正文的源码消息索引（流式分片会横跨多条） */
  indexes: number[];
}

export interface ThinkingSegment {
  type: 'thinking';
  thinking: string;
  originalIndex: number;
  indexes: number[];
}

export interface WidgetSegment {
  type: 'widget';
  widgetType: 'svg' | 'html';
  title: string;
  code: string;
  originalIndex: number;
  partIndex?: number;
}

export type AssistantSegment = TextSegment | ThinkingSegment | ToolGroupSegment | WidgetSegment;

export interface TurnNode {
  id: string;
  userMessage?: {
    msg: ChatMessage;
    originalIndex: number;
  };
  assistantTurn?: {
    model: string | null;
    timestamp: string;
    token_usage?: TokenUsage | null;
    segments: AssistantSegment[];
    originalIndexes: number[];
  };
}
