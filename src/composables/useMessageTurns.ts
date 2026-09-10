import { computed, type Ref } from "vue";
import type { ChatMessage, ContentPart } from "../types/session";
import { cleanUserText } from "../utils/textClean";
import { extractWidgetData } from "../utils/svg";
import type {
  TurnNode,
  ToolGroupSegment,
  TextSegment,
  ThinkingSegment,
  WidgetSegment,
} from "../types/chatTurn";

export function useMessageTurns(messages: Ref<ChatMessage[]> | Ref<Array<{msg: ChatMessage, originalIndex: number}>>) {
  const turns = computed<TurnNode[]>(() => {
    const result: TurnNode[] = [];
    let currentTurn: TurnNode | null = null;
    let toolUseMap = new Map<string, string>();
    const renderedWidgetIds = new Set<string>();

    const flushCurrentTurn = () => {
      if (currentTurn) {
        if (
          currentTurn.userMessage ||
          (currentTurn.assistantTurn && currentTurn.assistantTurn.segments.length > 0)
        ) {
          result.push(currentTurn);
        }
        currentTurn = null;
      }
    };

    messages.value.forEach((item, rawIdx) => {
      const msg = 'msg' in item ? item.msg : item;
      const idx = 'originalIndex' in item ? item.originalIndex : rawIdx;

      const parts = msg.content_parts;
      // is_meta 行（图片引用占位、local-command caveat 等）不参与用户卡片判定，
      // 避免将相邻 assistant 回复错误并入上一条 user Turn；引用行本身不会被渲染。
      // 用户文本以 cleanUserText 清洗后为准：纯 <command-name> 等系统标签行清洗后为空，
      // 不应产生空气泡。
      const hasUserTextInput = !msg.is_meta && msg.role === "user" && parts.some(
        (part) => (part.type === "text" && cleanUserText(part.text).length > 0) || part.type === "image"
      );
      const isToolResultOnlyUserMsg = msg.role === "user" && !msg.is_meta && !hasUserTextInput && parts.some(
        (part) => part.type === "tool_result"
      );

      if (isToolResultOnlyUserMsg) {
        // tool_result-only user payload → append to current assistant turn's tool group
        if (!currentTurn) {
          currentTurn = { id: `turn-${idx}` };
          toolUseMap = new Map<string, string>();
        }
        if (!currentTurn.assistantTurn) {
          currentTurn.assistantTurn = {
            model: msg.model,
            timestamp: msg.timestamp,
            segments: [],
            originalIndexes: [],
          };
        }
        currentTurn.assistantTurn.originalIndexes.push(idx);
        appendToolParts(currentTurn, idx, msg, toolUseMap, renderedWidgetIds);
        return;
      }

      if (msg.role === "user") {
        const imageParts = parts.filter((part) => part.type === "image" || part.type === "image_ref");
        const hasImage = imageParts.length > 0;

        if (msg.is_meta) {
          if (hasImage && currentTurn && currentTurn.userMessage) {
            const existingParts = currentTurn.userMessage.msg.content_parts;
            const newImages = imageParts.filter((imgPart) => {
              if (imgPart.type === "image_ref") {
                return !existingParts.some((p) => p.type === "image_ref" && p.path === imgPart.path);
              }
              if (imgPart.type === "image") {
                return !existingParts.some((p) => p.type === "image" && p.data === imgPart.data);
              }
              return true;
            });
            if (newImages.length > 0) {
              currentTurn.userMessage = {
                msg: {
                  ...currentTurn.userMessage.msg,
                  content_parts: [...existingParts, ...newImages],
                },
                originalIndex: currentTurn.userMessage.originalIndex,
              };
            }
          }
          return;
        }

        const hasUserText = parts.some((part) => part.type === "text" && cleanUserText(part.text).length > 0);

        if (hasImage && currentTurn && currentTurn.userMessage && !currentTurn.assistantTurn) {
          const existingParts = currentTurn.userMessage.msg.content_parts;
          const newImages = imageParts.filter((imgPart) => {
            if (imgPart.type === "image_ref") {
              return !existingParts.some((p) => p.type === "image_ref" && p.path === imgPart.path);
            }
            if (imgPart.type === "image") {
              return !existingParts.some((p) => p.type === "image" && p.data === imgPart.data);
            }
            return true;
          });
          if (newImages.length > 0) {
            currentTurn.userMessage = {
              msg: {
                ...currentTurn.userMessage.msg,
                content_parts: [...existingParts, ...newImages],
              },
              originalIndex: currentTurn.userMessage.originalIndex,
            };
          }
          if (!hasUserText) {
            return;
          }
        }

        if (!hasUserText && !hasImage) {
          return;
        }

        flushCurrentTurn();
        const filteredParts = parts.filter((p) => p.type !== "image_meta");

        currentTurn = {
          id: `turn-${idx}`,
          userMessage: {
            msg: {
              ...msg,
              content_parts: filteredParts,
            },
            originalIndex: idx,
          },
        };
        toolUseMap = new Map<string, string>();
        return;
      }

      // assistant message：每条消息各自独立成回合（企业微信式，同方连续消息不合并）
      flushCurrentTurn();
      currentTurn = {
        id: `turn-${idx}`,
        assistantTurn: {
          model: msg.model,
          timestamp: msg.timestamp,
          token_usage: msg.token_usage ? { ...msg.token_usage } : null,
          segments: [],
          originalIndexes: [idx],
        },
      };
      toolUseMap = new Map<string, string>();

      msg.content_parts.forEach((part: ContentPart, partIdx: number) => {
        const segments = currentTurn!.assistantTurn!.segments;
        const lastSegment = segments.length > 0 ? segments[segments.length - 1] : null;

        if (part.type === "text") {
          // 仅合并同一条原始消息内的多段文本，跨消息不合并：
          // 否则隐藏工具后原本被工具隔开的正文消息会合并成一个气泡，看起来"正文消失"
          if (lastSegment && lastSegment.type === "text" && lastSegment.originalIndex === idx) {
            lastSegment.text += part.text;
          } else {
            segments.push({
              type: "text",
              text: part.text,
              originalIndex: idx,
              partIndex: partIdx,
              indexes: [idx],
            } as TextSegment);
          }
        } else if (part.type === "thinking") {
          if (lastSegment && lastSegment.type === "thinking" && lastSegment.originalIndex === idx) {
            lastSegment.thinking += part.thinking;
          } else {
            segments.push({
              type: "thinking",
              thinking: part.thinking,
              originalIndex: idx,
              indexes: [idx],
            } as ThinkingSegment);
          }
        } else if (part.type === "tool_use" || part.type === "tool_result") {
          // Check if this part represents a visual widget (e.g. WorkBuddy show_widget or SVG)
          if (part.type === "tool_use") {
            const widget = extractWidgetData(part.input);
            if (widget || part.tool_name === "show_widget") {
              const widgetData = widget || {
                title: part.summary ? part.summary.replace(/^\[(?:图表|show_widget):\s*|\]$/g, "").trim() || "图表" : "图表",
                widgetCode: part.input,
                widgetType: "svg" as const,
              };
              if (part.tool_use_id) {
                renderedWidgetIds.add(part.tool_use_id);
              }
              segments.push({
                type: "widget",
                widgetType: widgetData.widgetType,
                title: widgetData.title,
                code: widgetData.widgetCode,
                originalIndex: idx,
                partIndex: partIdx,
              } as WidgetSegment);
              return;
            }
          } else if (part.type === "tool_result") {
            const isWidgetAck = (part.tool_use_id && renderedWidgetIds.has(part.tool_use_id)) ||
              (part.content && part.content.includes("visualizer_show_widget_result"));
            const isError = Boolean(part.is_error) || (part.summary && part.summary.includes("[Tool Error]"));
            if (isWidgetAck && !isError) {
              return;
            }
            const resultWidget = extractWidgetData(part.content);
            if (resultWidget && !isError) {
              if (part.tool_use_id) {
                renderedWidgetIds.add(part.tool_use_id);
              }
              segments.push({
                type: "widget",
                widgetType: resultWidget.widgetType,
                title: resultWidget.title,
                code: resultWidget.widgetCode,
                originalIndex: idx,
                partIndex: partIdx,
              } as WidgetSegment);
              return;
            }
          }

          if (part.type === "tool_use" && part.tool_use_id) {
            toolUseMap.set(part.tool_use_id, part.tool_name);
          }

          let toolGroupSeg = lastSegment && lastSegment.type === "tool_group"
            ? lastSegment
            : null;

          if (!toolGroupSeg) {
            toolGroupSeg = {
              type: "tool_group",
              groups: [],
              hasError: false,
            } as ToolGroupSegment;
            segments.push(toolGroupSeg);
          }

          let toolName = "Result";
          if (part.type === "tool_use") {
            toolName = part.tool_name;
          } else if (part.type === "tool_result") {
            if (part.tool_use_id && toolUseMap.has(part.tool_use_id)) {
              toolName = toolUseMap.get(part.tool_use_id)!;
            }
          }

          const summary = part.summary || "";
          const content = part.type === "tool_result" ? part.content : "";
          const isError =
            (part.type === "tool_result" && Boolean(part.is_error)) ||
            summary.includes("[Tool Error]") ||
            (part.type === "tool_result" && Boolean(content && content.includes("[Tool Error]")));

          if (isError) {
            toolGroupSeg.hasError = true;
          }

          let group = toolGroupSeg.groups.find((g) => g.tool_name === toolName);
          if (!group) {
            group = {
              tool_name: toolName,
              count: 0,
              hasError: false,
              items: [],
            };
            toolGroupSeg.groups.push(group);
          }

          if (isError) {
            group.hasError = true;
          }
          group.count += 1;
          group.items.push({
            id: `tool-${idx}-${group.items.length}`,
            tool_name: toolName,
            summary,
            input: part.type === "tool_use" ? part.input : part.content,
            is_error: isError,
            tool_use_id: part.type === "tool_use" ? part.tool_use_id : (part.type === "tool_result" ? part.tool_use_id : undefined),
            originalIndex: idx,
            partIndex: partIdx,
          });
        }
      });
    });

    flushCurrentTurn();
    return result;
  });

  return { turns };
}

function appendToolParts(
  currentTurn: TurnNode,
  idx: number,
  msg: ChatMessage,
  toolUseMap: Map<string, string>,
  renderedWidgetIds?: Set<string>
) {
  msg.content_parts.forEach((part: ContentPart, partIdx: number) => {
    if (part.type !== "tool_use" && part.type !== "tool_result") return;

    const segments = currentTurn.assistantTurn!.segments;
    const lastSegment = segments.length > 0 ? segments[segments.length - 1] : null;

    // Check if this part represents a visual widget
    if (part.type === "tool_use") {
      const widget = extractWidgetData(part.input);
      if (widget || part.tool_name === "show_widget") {
        const widgetData = widget || {
          title: part.summary ? part.summary.replace(/^\[(?:图表|show_widget):\s*|\]$/g, "").trim() || "图表" : "图表",
          widgetCode: part.input,
          widgetType: "svg" as const,
        };
        if (part.tool_use_id && renderedWidgetIds) {
          renderedWidgetIds.add(part.tool_use_id);
        }
        segments.push({
          type: "widget",
          widgetType: widgetData.widgetType,
          title: widgetData.title,
          code: widgetData.widgetCode,
          originalIndex: idx,
          partIndex: partIdx,
        } as WidgetSegment);
        return;
      }
    } else if (part.type === "tool_result") {
      const isWidgetAck = (part.tool_use_id && renderedWidgetIds?.has(part.tool_use_id)) ||
        (part.content && part.content.includes("visualizer_show_widget_result"));
      const isError = Boolean(part.is_error) || (part.summary && part.summary.includes("[Tool Error]"));
      if (isWidgetAck && !isError) {
        return;
      }
      const resultWidget = extractWidgetData(part.content);
      if (resultWidget && !isError) {
        if (part.tool_use_id && renderedWidgetIds) {
          renderedWidgetIds.add(part.tool_use_id);
        }
        segments.push({
          type: "widget",
          widgetType: resultWidget.widgetType,
          title: resultWidget.title,
          code: resultWidget.widgetCode,
          originalIndex: idx,
          partIndex: partIdx,
        } as WidgetSegment);
        return;
      }
    }

    if (part.type === "tool_use" && part.tool_use_id) {
      toolUseMap.set(part.tool_use_id, part.tool_name);
    }

    let toolGroupSeg = lastSegment && lastSegment.type === "tool_group"
      ? lastSegment
      : null;

    if (!toolGroupSeg) {
      toolGroupSeg = {
        type: "tool_group",
        groups: [],
        hasError: false,
      } as ToolGroupSegment;
      segments.push(toolGroupSeg);
    }

    const content = part.type === "tool_result" ? part.content : "";
    let toolName = "Result";
    if (part.type === "tool_use") {
      toolName = part.tool_name;
    } else if (part.type === "tool_result") {
      if (part.tool_use_id && toolUseMap.has(part.tool_use_id)) {
        toolName = toolUseMap.get(part.tool_use_id)!;
      } else if (content.includes("Base directory for this skill:")) {
        const skillMatch = content.match(/#\s+([A-Za-z0-9 _-]+)/);
        toolName = skillMatch ? `Skill: ${skillMatch[1].trim()}` : "Skill";
      }
    }

    const summary = part.summary || "";
    const isError =
      (part.type === "tool_result" && Boolean(part.is_error)) ||
      summary.includes("[Tool Error]") ||
      (part.type === "tool_result" && Boolean(content && content.includes("[Tool Error]")));

    if (isError) {
      toolGroupSeg.hasError = true;
    }

    let group = toolGroupSeg.groups.find((g) => g.tool_name === toolName);
    if (!group) {
      group = {
        tool_name: toolName,
        count: 0,
        hasError: false,
        items: [],
      };
      toolGroupSeg.groups.push(group);
    }

    if (isError) {
      group.hasError = true;
    }
    group.count += 1;
    group.items.push({
      id: `tool-${idx}-${group.items.length}`,
      tool_name: toolName,
      summary,
      input: part.type === "tool_use" ? part.input : part.content,
      is_error: isError,
      tool_use_id: part.type === "tool_use" ? part.tool_use_id : (part.type === "tool_result" ? part.tool_use_id : undefined),
      originalIndex: idx,
      partIndex: partIdx,
    });
  });
}
