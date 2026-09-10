export async function copyToClipboard(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Fallback for older browsers
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(textarea);
    return ok;
  }
}

/**
 * 复制异步产生的文本（如需要先 await 后端命令的结果）。
 *
 * 必须在用户手势激活期内同步调用：把 Promise 包进 ClipboardItem，
 * 让 clipboard.write() 同步发起、数据异步到达。旧版 WKWebView（旧 macOS）
 * 中 await 之后再 writeText 会因 transient activation 失效而被拒绝。
 * 注意 Promise 必须 resolve 为 Blob，直接给字符串在部分浏览器会静默写空。
 */
export function copyPromiseToClipboard(
  textPromise: Promise<string>
): Promise<boolean> {
  try {
    const item = new ClipboardItem({
      "text/plain": textPromise.then(
        (text) => new Blob([text], { type: "text/plain" })
      ),
    });
    return navigator.clipboard.write([item]).then(
      () => true,
      async () => {
        // ClipboardItem 路径失败（如不支持），降级为等待结果后走旧链路
        try {
          const text = await textPromise;
          return await copyToClipboard(text);
        } catch {
          return false;
        }
      }
    );
  } catch {
    return textPromise.then(
      (text) => copyToClipboard(text),
      () => false
    );
  }
}
