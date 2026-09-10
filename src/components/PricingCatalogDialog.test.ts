import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mount, flushPromises } from "@vue/test-utils";
import PricingCatalogDialog from "./PricingCatalogDialog.vue";
import { resetPricingCatalogStore, usePricingCatalog } from "../composables/usePricingCatalog";

describe("PricingCatalogDialog.vue", () => {
  let store: Record<string, string> = {};

  beforeEach(() => {
    store = {};
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => store[key] ?? null,
      setItem: (key: string, val: string) => {
        store[key] = val;
      },
      removeItem: (key: string) => {
        delete store[key];
      },
      clear: () => {
        store = {};
      },
    });
    resetPricingCatalogStore();
    vi.restoreAllMocks();
  });

  afterEach(() => {
    store = {};
    resetPricingCatalogStore();
    vi.restoreAllMocks();
  });

  function mountDialog() {
    return mount(PricingCatalogDialog, {
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
      attachTo: document.body,
    });
  }

  it("renders header with title, model count badge and update info", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    expect(wrapper.text()).toContain("模型价格目录");
    const countBadge = wrapper.find(".model-count-badge");
    expect(countBadge.exists()).toBe(true);
    expect(countBadge.text()).toMatch(/共\s+\d+\s+个模型/);
    expect(wrapper.text()).toContain("models.dev");
    wrapper.unmount();
  });

  it("renders pricing table rows with model, provider, and formatted rates", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);

    // Default catalog includes claude-sonnet-4-6
    expect(wrapper.text()).toContain("claude-sonnet-4-6");
    expect(wrapper.text()).toContain("Anthropic");
    wrapper.unmount();
  });

  it("filters models by search query matching model name, provider or key", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const searchInput = wrapper.find('input[type="text"]');
    expect(searchInput.exists()).toBe(true);

    // Search for deepseek
    await searchInput.setValue("deepseek");
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.text().toLowerCase()).toContain("deepseek");
    }

    // Search for non-existent model
    await searchInput.setValue("non-existent-model-xyz-12345");
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll("tbody tr").length).toBe(0);
    expect(wrapper.text()).toContain("未找到匹配的模型");
    wrapper.unmount();
  });

  it("filters models by provider select dropdown", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const providerSelect = wrapper.findComponent('[data-testid="provider-select"]');
    expect(providerSelect.exists()).toBe(true);

    // Select Anthropic
    await providerSelect.vm.$emit("update:modelValue", "Anthropic");
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBeGreaterThan(0);
    for (const row of rows) {
      expect(row.text()).toContain("Anthropic");
    }

    // Reset to all
    await providerSelect.vm.$emit("update:modelValue", "all");
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll("tbody tr").length).toBeGreaterThan(rows.length);
    wrapper.unmount();
  });

  it("filters to used models when usedModels prop is passed and supports scope switching", async () => {
    const wrapper = mount(PricingCatalogDialog, {
      props: {
        usedModels: ["claude-3-5-sonnet", "deepseek-v3"],
      },
      global: {
        stubs: {
          SvgIcon: true,
        },
      },
    });
    await flushPromises();

    const usedTab = wrapper.find('[data-testid="scope-used"]');
    expect(usedTab.exists()).toBe(true);
    expect(usedTab.classes()).toContain("active");

    const rows = wrapper.findAll("tbody tr");
    expect(rows.length).toBe(2);
    expect(wrapper.text()).toContain("claude-3-5-sonnet");
    expect(wrapper.text()).toContain("deepseek-v3");

    // Switch to all scope
    const allTab = wrapper.find('[data-testid="scope-all"]');
    expect(allTab.exists()).toBe(true);
    await allTab.trigger("click");
    await wrapper.vm.$nextTick();

    expect(allTab.classes()).toContain("active");
    expect(wrapper.findAll("tbody tr").length).toBeGreaterThan(2);
    wrapper.unmount();
  });

  it("emits close when clicking close button, overlay backdrop or pressing Escape", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    // 1. Close button
    const closeBtn = wrapper.find(".close-btn");
    expect(closeBtn.exists()).toBe(true);
    await closeBtn.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(1);

    // 2. Overlay backdrop click
    const overlay = wrapper.find(".pricing-overlay");
    await overlay.trigger("click");
    expect(wrapper.emitted("close")).toHaveLength(2);

    // 3. Escape keydown
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wrapper.vm.$nextTick();
    expect(wrapper.emitted("close")).toHaveLength(3);

    wrapper.unmount();
  });

  it("calls fetchPricingCatalog(true) when clicking refresh button", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce({
      ok: true,
      json: async () => ({}),
    } as unknown as Response);

    const wrapper = mountDialog();
    await flushPromises();

    const refreshBtn = wrapper.find(".refresh-btn");
    expect(refreshBtn.exists()).toBe(true);
    await refreshBtn.trigger("click");
    await flushPromises();

    expect(fetchSpy).toHaveBeenCalledWith("https://models.dev/api.json");
    wrapper.unmount();
  });

  it("displays loading/spinning state when isRefreshing is true", async () => {
    const store = usePricingCatalog();
    store.isRefreshing.value = true;

    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    const refreshBtn = wrapper.find(".refresh-btn");
    expect(refreshBtn.classes()).toContain("is-spinning");
    expect(refreshBtn.attributes("disabled")).toBeDefined();

    store.isRefreshing.value = false;
    wrapper.unmount();
  });

  it("clears search input when clicking clear button", async () => {
    const wrapper = mountDialog();
    await flushPromises();

    const searchInput = wrapper.find('input[type="text"]');
    await searchInput.setValue("claude");
    await wrapper.vm.$nextTick();

    const clearBtn = wrapper.find(".clear-search-btn");
    expect(clearBtn.exists()).toBe(true);

    await clearBtn.trigger("click");
    await wrapper.vm.$nextTick();

    expect((searchInput.element as HTMLInputElement).value).toBe("");
    expect(wrapper.find(".clear-search-btn").exists()).toBe(false);
    wrapper.unmount();
  });

  it("displays refresh error banner when refreshError is set", async () => {
    const store = usePricingCatalog();
    store.refreshError.value = "Network connection timeout";

    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    const errorBanner = wrapper.find(".refresh-error-banner");
    expect(errorBanner.exists()).toBe(true);
    expect(errorBanner.text()).toContain("Network connection timeout");

    store.refreshError.value = "";
    wrapper.unmount();
  });
});
