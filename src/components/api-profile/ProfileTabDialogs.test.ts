import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import ProfileTabCreateDialog from "./ProfileTabCreateDialog.vue";
import ProfileTabEditDialog from "./ProfileTabEditDialog.vue";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

vi.mock("../../composables/useSessions", () => ({
  useSessions: () => ({
    projects: {
      value: [
        { original_path: "/workspace/valid-project" },
        { original_path: "/workspace/missing-project" },
      ],
    },
  }),
}));

describe("ProfileTabDialogs - 本地目录失效检测与 Finder 打开错误处理", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    Object.assign(navigator, {
      clipboard: {
        writeText: vi.fn().mockResolvedValue(undefined),
      },
    });
  });

  describe("ProfileTabCreateDialog", () => {
    it("检测到不存在的目录时显示「已失效」徽标与失效样式", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd: string, _args: any) => {
        if (cmd === "check_paths_exist") {
          return {
            "/workspace/valid-project": true,
            "/workspace/missing-project": false,
          };
        }
        return [];
      });

      const wrapper = mount(ProfileTabCreateDialog, {
        props: { existingNames: [] },
      });

      await flushPromises();

      const items = wrapper.findAll(".project-item");
      expect(items.length).toBe(2);

      const missingItem = items.find((el) => el.text().includes("missing-project"));
      expect(missingItem).toBeDefined();
      expect(missingItem?.classes()).toContain("is-missing");
      expect(missingItem?.find(".missing-badge").exists()).toBe(true);
      expect(missingItem?.find(".missing-badge").text()).toBe("已失效");

      const validItem = items.find((el) => el.text().includes("valid-project"));
      expect(validItem?.classes()).not.toContain("is-missing");
      expect(validItem?.find(".missing-badge").exists()).toBe(false);
    });

    it("点击通过 Finder 打开不存在的目录时，给出错误通知并自动复制路径", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd: string, args: any) => {
        if (cmd === "check_paths_exist") {
          return {
            "/workspace/valid-project": true,
            "/workspace/missing-project": false,
          };
        }
        if (cmd === "open_path_in_file_manager") {
          if (args?.path === "/workspace/missing-project") {
            return Promise.reject("路径不存在：/workspace/missing-project");
          }
          return Promise.resolve();
        }
        return [];
      });

      const wrapper = mount(ProfileTabCreateDialog, {
        props: { existingNames: [] },
      });

      await flushPromises();

      const missingItem = wrapper.findAll(".project-item").find((el) => el.text().includes("missing-project"));
      const openBtn = missingItem?.findAll(".project-action-btn")[1];
      expect(openBtn).toBeDefined();

      await openBtn?.trigger("click");
      await flushPromises();

      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("/workspace/missing-project");

      const notice = wrapper.find(".action-notice");
      expect(notice.exists()).toBe(true);
      expect(notice.classes()).toContain("is-error");
      expect(notice.text()).toContain("打开失败：本地目录已不存在或已被移除（已自动复制路径）");
    });
  });

  describe("ProfileTabEditDialog", () => {
    const tabMock = {
      id: "tab-1",
      cli_id: "claude",
      name: "TestScope",
      sort_order: 1,
      dirs: ["/workspace/missing-project"],
    };

    it("检测到绑定的失效目录时显示「已失效」徽标并在打开失败时提示", async () => {
      vi.mocked(invoke).mockImplementation(async (cmd: string, _args: any) => {
        if (cmd === "check_paths_exist") {
          return {
            "/workspace/valid-project": true,
            "/workspace/missing-project": false,
          };
        }
        if (cmd === "open_path_in_file_manager") {
          return Promise.reject("路径不存在：/workspace/missing-project");
        }
        return [];
      });

      const wrapper = mount(ProfileTabEditDialog, {
        props: { tab: tabMock, existingNames: [] },
      });

      await flushPromises();

      const missingItem = wrapper.findAll(".project-item").find((el) => el.text().includes("missing-project"));
      expect(missingItem?.classes()).toContain("is-missing");
      expect(missingItem?.find(".missing-badge").text()).toBe("已失效");

      const openBtn = missingItem?.findAll(".project-action-btn")[1];
      await openBtn?.trigger("click");
      await flushPromises();

      expect(navigator.clipboard.writeText).toHaveBeenCalledWith("/workspace/missing-project");
      const notice = wrapper.find(".action-notice");
      expect(notice.exists()).toBe(true);
      expect(notice.text()).toContain("打开失败：本地目录已不存在或已被移除（已自动复制路径）");
    });
  });
});
