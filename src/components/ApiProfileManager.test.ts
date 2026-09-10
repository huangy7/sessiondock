import { describe, it, expect, vi, beforeEach } from "vitest";
import { shallowMount, flushPromises } from "@vue/test-utils";
import { ref } from "vue";
import ApiProfileManager from "./ApiProfileManager.vue";
import ProfileImportConflictDialog from "./api-profile/ProfileImportConflictDialog.vue";
import { CLI_DEFINITIONS, type CliId, type CliOption } from "../types/cli";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  ask: vi.fn(),
  message: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
  copyPromiseToClipboard: vi.fn(),
  readText: vi.fn(),
  addProxyStatusChangedListener: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: mocks.ask,
  message: mocks.message,
  open: mocks.open,
  save: mocks.save,
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: mocks.readText,
}));
vi.mock("../utils/clipboard", () => ({
  copyPromiseToClipboard: mocks.copyPromiseToClipboard,
}));
vi.mock("../composables/useProxy", () => ({
  addProxyStatusChangedListener: mocks.addProxyStatusChangedListener,
}));

const mockCliOptions: CliOption[] = (Object.keys(CLI_DEFINITIONS) as CliId[]).map((id) => ({
  ...CLI_DEFINITIONS[id],
  hasSessions: true,
  hasBinary: true,
}));

vi.mock("../composables/useSessions", () => ({
  useSessions: () => ({
    cliOptions: ref(mockCliOptions),
  }),
}));

let localStore: Record<string, string> = {};
vi.stubGlobal("localStorage", {
  getItem: (key: string) => localStore[key] ?? null,
  setItem: (key: string, val: string) => { localStore[key] = val; },
  removeItem: (key: string) => { delete localStore[key]; },
  clear: () => { localStore = {}; },
});

/** mount 期各命令的默认响应 */
function defaultResponse(cmd: string): unknown {
  switch (cmd) {
    case "list_profile_tabs":
    case "list_profiles":
    case "get_scope_bindings":
    case "get_scope_dir_status":
      return [];
    case "read_scope_settings":
      return "{}";
    case "proxy_status":
      return { enabled: false, running: false, port: 18080 };
    default:
      return null;
  }
}

function mockInvokeWith(overrides: Record<string, unknown>) {
  mocks.invoke.mockImplementation(async (cmd: string) => {
    if (cmd in overrides) {
      const value = overrides[cmd];
      if (value instanceof Error) throw value;
      return value;
    }
    return defaultResponse(cmd);
  });
}

async function mountManager() {
  const wrapper = shallowMount(ApiProfileManager);
  await flushPromises();
  return wrapper;
}

type ManagerWrapper = Awaited<ReturnType<typeof mountManager>>;

async function openMenu(wrapper: ManagerWrapper, index: 0 | 1) {
  const btn = wrapper.findAll(".header-actions .btn-header-tool")[index];
  await btn.trigger("click");
}

async function clickMenuItem(wrapper: ManagerWrapper, label: string) {
  const item = wrapper.findAll(".backup-menu button").find((b) => b.text() === label);
  expect(item, `菜单项 ${label} 应存在`).toBeDefined();
  await item!.trigger("click");
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.invoke.mockImplementation(async (cmd: string) => defaultResponse(cmd));
  mocks.listen.mockResolvedValue(() => {});
  mocks.addProxyStatusChangedListener.mockReturnValue(() => {});
  mocks.copyPromiseToClipboard.mockImplementation((p: Promise<string>) =>
    p.then(() => true),
  );
});

describe("ApiProfileManager 备份菜单", () => {
  it("导入/导出按钮展开下拉菜单，各含文件与剪贴板两个通道", async () => {
    const wrapper = await mountManager();

    await openMenu(wrapper, 0);
    let items = wrapper.findAll(".backup-menu button").map((b) => b.text());
    expect(items).toEqual(["从文件导入…", "从剪贴板导入"]);

    await openMenu(wrapper, 1);
    items = wrapper.findAll(".backup-menu button").map((b) => b.text());
    expect(items).toEqual(["导出到文件…", "复制到剪贴板"]);
  });

  it("点击组件外部后菜单关闭", async () => {
    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    expect(wrapper.find(".backup-menu").exists()).toBe(true);

    document.body.click();
    await wrapper.vm.$nextTick();
    expect(wrapper.find(".backup-menu").exists()).toBe(false);
  });
});

describe("ApiProfileManager 导出到剪贴板", () => {
  it("确认警告后导出全部配置并写入剪贴板（不传 savePath）", async () => {
    mocks.ask.mockResolvedValue(true);
    const backupData = { version: 1, timestamp: "t", profiles: [], tabs: [] };
    mockInvokeWith({ export_profiles_backup: backupData });

    const wrapper = await mountManager();
    await openMenu(wrapper, 1);
    await clickMenuItem(wrapper, "复制到剪贴板");
    await flushPromises();

    expect(mocks.ask).toHaveBeenCalledOnce();
    expect(mocks.invoke).toHaveBeenCalledWith("export_profiles_backup", {});
    expect(mocks.copyPromiseToClipboard).toHaveBeenCalledOnce();
    const passed = mocks.copyPromiseToClipboard.mock.calls[0][0] as Promise<string>;
    await expect(passed).resolves.toBe(JSON.stringify(backupData, null, 2));
  });

  it("取消警告后不发起导出", async () => {
    mocks.ask.mockResolvedValue(false);
    const wrapper = await mountManager();
    await openMenu(wrapper, 1);
    await clickMenuItem(wrapper, "复制到剪贴板");
    await flushPromises();

    expect(mocks.invoke).not.toHaveBeenCalledWith("export_profiles_backup", expect.anything());
    expect(mocks.copyPromiseToClipboard).not.toHaveBeenCalled();
  });
});

describe("ApiProfileManager 从剪贴板导入", () => {
  it("剪贴板为空时提示且不调解析命令", async () => {
    mocks.readText.mockResolvedValue("   ");
    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, "从剪贴板导入");
    await flushPromises();

    expect(mocks.message).toHaveBeenCalledWith(
      expect.stringContaining("剪贴板为空"),
      expect.objectContaining({ kind: "warning" }),
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("parse_profiles_backup", expect.anything());
  });

  it("剪贴板内容非法时提示导入失败", async () => {
    mocks.readText.mockResolvedValue("not json");
    mockInvokeWith({ parse_profiles_backup: new Error("解析备份内容失败: ...") });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, "从剪贴板导入");
    await flushPromises();

    expect(mocks.message).toHaveBeenCalledWith(
      expect.stringContaining("不是有效的配置备份"),
      expect.objectContaining({ kind: "error" }),
    );
    expect(mocks.invoke).not.toHaveBeenCalledWith("import_profiles_backup", expect.anything());
  });

  it("合法备份且无冲突时直接执行导入", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [
        { cli_id: "claude", scope: "global", name: "test", content: "{}", is_active: false },
      ],
      tabs: [],
    };
    mockInvokeWith({ parse_profiles_backup: backupData });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, "从剪贴板导入");
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith("parse_profiles_backup", {
      content: '{"version":1}',
    });
    expect(mocks.invoke).toHaveBeenCalledWith(
      "import_profiles_backup",
      expect.objectContaining({
        items: [expect.objectContaining({ name: "test", action: "overwrite" })],
      }),
    );
  });

  it("备份中的作用域绑定随导入透传给后端", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [
        { cli_id: "claude", scope: "global", name: "kimi", content: "{}", is_active: false },
      ],
      tabs: [],
      scope_bindings: [
        { cli_id: "claude", scope: "OYxV4yFhv6", profile_name: "kimi" },
      ],
    };
    mockInvokeWith({ parse_profiles_backup: backupData });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, "从剪贴板导入");
    await flushPromises();

    expect(mocks.invoke).toHaveBeenCalledWith(
      "import_profiles_backup",
      expect.objectContaining({
        scopeBindings: [
          { cli_id: "claude", scope: "OYxV4yFhv6", profile_name: "kimi" },
        ],
      }),
    );
  });

  it("作用域同名不同 id 触发冲突，跳过的作用域不传给后端", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [{ cli_id: "claude", scope: "global", name: "a", content: "{}", is_active: false }],
      tabs: [{ id: "t1", cli_id: "claude", name: "scope-x", dirs: ["/d"] }],
      scope_bindings: [{ cli_id: "claude", scope: "t1", profile_name: "a" }],
    };
    mocks.invoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "parse_profiles_backup") return backupData;
      // 本地有一个同名的 scope-x，但 id 不同（t-local）
      if (cmd === "list_profile_tabs") {
        return [{ id: "t-local", cli_id: "claude", name: "scope-x", dirs: [] }];
      }
      return defaultResponse(cmd);
    });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, "从剪贴板导入");
    await flushPromises();

    // 同名不同 id → 触发对话框，作用域状态为重名冲突、默认跳过
    const dialog = wrapper.findComponent(ProfileImportConflictDialog);
    expect(dialog.exists()).toBe(true);
    const scopes = dialog.props("scopes") as Array<{ id: string; name: string; status: string; action: string }>;
    expect(scopes[0].status).toBe("nameConflict");
    expect(scopes[0].action).toBe("skip");

    // 模拟用户在对话框里保持默认（跳过该作用域），确认导入
    await dialog.vm.$emit("confirm", [], scopes);
    await flushPromises();
    const call = mocks.invoke.mock.calls.find((c) => c[0] === "import_profiles_backup");
    expect(call).toBeDefined();
    // 跳过的作用域不应进入 tabs 载荷
    expect((call![1] as any).tabs).toEqual([]);
  });

  it("冲突检测按备份中各 CLI 的本地配置逐一比对（多 CLI 备份）", async () => {
    mocks.readText.mockResolvedValue('{"version":1}');
    const backupData = {
      version: 1,
      timestamp: "t",
      profiles: [
        { cli_id: "claude", scope: "global", name: "claude", content: "{}", is_active: false },
        { cli_id: "codex", scope: "global", name: "default", content: "{}", is_active: false },
      ],
      tabs: [],
    };
    mocks.invoke.mockImplementation(async (cmd: string, args?: any) => {
      if (cmd === "parse_profiles_backup") return backupData;
      // 本地 claude 有 "claude"，codex 有 "default" —— 两条都应判为冲突
      if (cmd === "list_profiles") return args?.cliId === "codex" ? ["default"] : ["claude"];
      return defaultResponse(cmd);
    });

    const wrapper = await mountManager();
    await openMenu(wrapper, 0);
    await clickMenuItem(wrapper, "从剪贴板导入");
    await flushPromises();

    const dialog = wrapper.findComponent(ProfileImportConflictDialog);
    expect(dialog.exists()).toBe(true);
    const candidates = dialog.props("candidates") as Array<{ name: string; isConflict: boolean }>;
    expect(candidates).toHaveLength(2);
    expect(candidates.find((c) => c.name === "claude")?.isConflict).toBe(true);
    expect(candidates.find((c) => c.name === "default")?.isConflict).toBe(true);
  });
});
