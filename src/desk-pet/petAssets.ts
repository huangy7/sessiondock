import idleFollowUrl from "../assets/desk-pet/clawd/clawd-idle-follow.svg?url";
import reactDragUrl from "../assets/desk-pet/clawd/clawd-react-drag.svg?url";
import reactLeftUrl from "../assets/desk-pet/clawd/clawd-react-left.svg?url";
import reactRightUrl from "../assets/desk-pet/clawd/clawd-react-right.svg?url";
import reactAnnoyedUrl from "../assets/desk-pet/clawd/clawd-react-annoyed.svg?url";
import reactDoubleUrl from "../assets/desk-pet/clawd/clawd-react-double.svg?url";
import reactDoubleJumpUrl from "../assets/desk-pet/clawd/clawd-react-double-jump.svg?url";
import miniIdleUrl from "../assets/desk-pet/clawd/clawd-mini-idle.svg?url";
import miniPeekUrl from "../assets/desk-pet/clawd/clawd-mini-peek.svg?url";
import dizzyUrl from "../assets/desk-pet/clawd/clawd-dizzy.svg?url";
import workingTypingUrl from "../assets/desk-pet/clawd/clawd-working-typing.svg?url";
import happyUrl from "../assets/desk-pet/clawd/clawd-happy.svg?url";

const ASSET_URLS: Record<string, string> = {
  "clawd-idle-follow.svg": idleFollowUrl,
  "clawd-react-drag.svg": reactDragUrl,
  "clawd-react-left.svg": reactLeftUrl,
  "clawd-react-right.svg": reactRightUrl,
  "clawd-react-annoyed.svg": reactAnnoyedUrl,
  "clawd-react-double.svg": reactDoubleUrl,
  "clawd-react-double-jump.svg": reactDoubleJumpUrl,
  "clawd-mini-idle.svg": miniIdleUrl,
  "clawd-mini-peek.svg": miniPeekUrl,
  "clawd-dizzy.svg": dizzyUrl,
  "clawd-working-typing.svg": workingTypingUrl,
  "clawd-happy.svg": happyUrl,
};

export const IDLE_FALLBACK_URL = idleFollowUrl;
export const MINI_IDLE_URL = miniIdleUrl;
export const MINI_PEEK_URL = miniPeekUrl;

/** 内联 SVG 通道（SVG DOM 直接可操作以做眼睛跟随）的资源 */
export const INLINE_SVG_URLS: ReadonlySet<string> = new Set([idleFollowUrl, miniIdleUrl]);

const markupCache = new Map<string, Promise<string>>();

/** 拉取 SVG 文本（带缓存），供内联渲染与眼睛跟随 DOM 操作 */
export function fetchSvgMarkup(url: string): Promise<string> {
  let cached = markupCache.get(url);
  if (!cached) {
    cached = fetch(url).then((r) => {
      if (!r.ok) throw new Error(`fetch SVG failed: ${url} (${r.status})`);
      return r.text();
    });
    markupCache.set(url, cached);
  }
  return cached;
}

export function petAssetUrl(file: string): string | undefined {
  return ASSET_URLS[file];
}
