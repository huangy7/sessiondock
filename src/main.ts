import { createApp } from "vue";
import { message } from "@tauri-apps/plugin-dialog";
import "./styles/utilities.css";
import "./styles/menu.css";
import App from "./App.vue";

const APP_RUNNING_KEY = "claudia_app_running";
if (sessionStorage.getItem(APP_RUNNING_KEY) === "true") {
  setTimeout(() => window.dispatchEvent(new CustomEvent("app-crash-recovery")), 1000);
}
sessionStorage.setItem(APP_RUNNING_KEY, "true");

window.addEventListener("beforeunload", () => {
  sessionStorage.removeItem(APP_RUNNING_KEY);
});

window.addEventListener("unhandledrejection", (event) => {
  console.error("Unhandled Promise Rejection:", event.reason);
});

import { vMermaid } from "./directives/vMermaid";

const app = createApp(App);
app.directive("mermaid", vMermaid);

app.config.errorHandler = (err, instance, info) => {
  const stack = err instanceof Error && err.stack ? err.stack : String(err);
  const chain: string[] = [];
  let cur = instance;
  while (cur) {
    const t = (cur as unknown as { type: { name?: string; __file?: string } }).type;
    chain.push(t?.__file ?? t?.name ?? "Anonymous");
    cur = cur.$parent;
  }
  console.error("Global Vue Error:", err, info, stack, chain);
  const detail = `${err}\n\n组件位置: ${info}\n组件链: ${chain.join(" → ") || "未知"}\n\n堆栈:\n${stack.split("\n").slice(0, 8).join("\n")}`;
  message(`发生渲染错误: ${detail}\n\n建议重启应用或联系开发者。`, { title: "渲染错误", kind: "error" }).catch(console.error);
};

app.mount("#app");

