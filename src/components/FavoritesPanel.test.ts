import { ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { FavoriteEntry } from "../composables/useFavorites";
import type { AggregatedProjectInfo, SessionInfo } from "../types/session";

const mocks = vi.hoisted(() => ({
  favorites: undefined as ReturnType<typeof ref<FavoriteEntry[]>> | undefined,
}));

vi.mock("../composables/useFavorites", async () => {
  const { ref } = await vi.importActual<typeof import("vue")>("vue");
  mocks.favorites = ref<FavoriteEntry[]>([]);
  return {
    useFavorites: () => ({
      favorites: mocks.favorites,
      loadFavorites: vi.fn(),
      isFavorite: () => false,
      toggleFavorite: vi.fn(),
    }),
  };
});

const { mount } = await import("@vue/test-utils");
const FavoritesPanel = (await import("./FavoritesPanel.vue")).default;

function makeSession(filePath: string, displayName: string, cliId = "claude"): SessionInfo {
  return {
    session_id: filePath.split("/").pop() ?? filePath,
    file_path: filePath,
    display_name: displayName,
    timestamp: "2026-08-27T08:00:00Z",
    file_size: 100,
    git_branch: "main",
    has_archive_snapshot: false,
    is_archived: false,
    cli_id: cliId,
  };
}

function makeProject(
  projectKey: string,
  originalPath: string,
  sessions: SessionInfo[],
): AggregatedProjectInfo {
  return {
    encoded_dir: `enc-${projectKey}`,
    original_path: originalPath,
    project_key: projectKey,
    cli_ids: ["claude"],
    sessions,
  };
}

const sessionA = makeSession("/data/projA/s1.jsonl", "修复登录 Bug");
const sessionB = makeSession("/data/projB/s2.jsonl", "重构侧边栏");
const projectA = makeProject("keyA", "/Users/me/projA", [sessionA]);
const projectB = makeProject("keyB", "/Users/me/projB", [sessionB]);

function mountPanel(projects: AggregatedProjectInfo[] = [projectA, projectB]) {
  return mount(FavoritesPanel, {
    props: { projects },
    global: { stubs: { SvgIcon: true } },
  });
}

beforeEach(() => {
  mocks.favorites!.value = [
    { cli_id: "claude", path: sessionA.file_path },
    { cli_id: "claude", path: sessionB.file_path },
  ];
});

describe("FavoritesPanel", () => {
  it("空列表显示引导文案", () => {
    mocks.favorites!.value = [];
    const wrapper = mountPanel();

    expect(wrapper.text()).toContain("暂无收藏对话");
    expect(wrapper.findAll(".session-item")).toHaveLength(0);
  });

  it("按项目分组渲染，标题含收藏与计数徽章", () => {
    const wrapper = mountPanel();

    expect(wrapper.text()).toContain("收藏");
    expect(wrapper.find(".count-badge").text()).toBe("2");

    const headers = wrapper.findAll(".project-header");
    expect(headers).toHaveLength(2);
    expect(headers.map((h) => h.text())).toEqual(
      expect.arrayContaining([expect.stringContaining("/Users/me/projA"), expect.stringContaining("/Users/me/projB")]),
    );

    const rows = wrapper.findAll(".session-item");
    expect(rows).toHaveLength(2);
    expect(wrapper.text()).toContain("修复登录 Bug");
    expect(wrapper.text()).toContain("重构侧边栏");
  });

  it("点击星标行 emit openSession，携带复合身份与 encodedDir", async () => {
    const wrapper = mountPanel();

    const row = wrapper.findAll(".session-item").find((r) => r.text().includes("修复登录 Bug"));
    expect(row).toBeTruthy();
    await row!.trigger("click");

    expect(wrapper.emitted("openSession")).toEqual([
      [{ cliId: "claude", filePath: sessionA.file_path }, "enc-keyA"],
    ]);
  });

  it("找不到对应会话的星标显示'源对话缺失'灰态且不可点击", async () => {
    mocks.favorites!.value = [
      ...mocks.favorites!.value,
      { cli_id: "claude", path: "/data/gone/deleted.jsonl" },
    ];
    const wrapper = mountPanel();

    expect(wrapper.text()).toContain("源对话缺失");

    const missingRow = wrapper
      .findAll(".session-item")
      .find((r) => r.text().includes("deleted.jsonl"));
    expect(missingRow).toBeTruthy();
    expect(missingRow!.classes()).toContain("favorite-missing");

    await missingRow!.trigger("click");
    expect(wrapper.emitted("openSession")).toBeUndefined();
  });

  it("星标条目按 cli_id + path 复合匹配，跨 CLI 同路径不误配", () => {
    const codexProject = makeProject("keyC", "/Users/me/projC", [
      makeSession(sessionA.file_path, "Codex 同名对话", "codex"),
    ]);
    mocks.favorites!.value = [{ cli_id: "codex", path: sessionA.file_path }];

    const wrapper = mountPanel([projectA, codexProject]);

    expect(wrapper.text()).toContain("Codex 同名对话");
    expect(wrapper.text()).not.toContain("修复登录 Bug");
  });
});
