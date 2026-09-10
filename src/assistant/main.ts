import { createApp } from "vue";
import "../styles/utilities.css";
// 模块级副作用：应用 data-theme + 监听主窗口主题广播（app-theme-changed / storage 双通道）
import "../composables/useTheme";
import AssistantApp from "./AssistantApp.vue";

window.addEventListener("unhandledrejection", (event) => {
  console.error("Unhandled Promise Rejection:", event.reason);
});

import { vMermaid } from "../directives/vMermaid";

const app = createApp(AssistantApp);
app.directive("mermaid", vMermaid);
app.mount("#app");
