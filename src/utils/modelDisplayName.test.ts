import { describe, expect, it } from "vitest";
import {
  backfillEmptyDisplayNames,
  buildDisplayById,
  parseCachedModelEntries,
  resolveAutoDisplayName,
  type ModelEntry,
} from "./modelDisplayName";

const entries: ModelEntry[] = [
  { id: "kimi-k2.6", display: "Kimi K2.6" },
  { id: "claude-sonnet-4-5", display: "Claude Sonnet 4.5" },
  { id: "no-display" },
];

describe("buildDisplayById", () => {
  it("maps id to display, skipping entries without display", () => {
    const map = buildDisplayById(entries);
    expect(map.get("kimi-k2.6")).toBe("Kimi K2.6");
    expect(map.get("claude-sonnet-4-5")).toBe("Claude Sonnet 4.5");
    expect(map.has("no-display")).toBe(false);
  });

  it("ignores empty or whitespace-only display", () => {
    const map = buildDisplayById([{ id: "a", display: "  " }, { id: "b", display: "" }]);
    expect(map.has("a")).toBe(false);
    expect(map.has("b")).toBe(false);
  });
});

describe("resolveAutoDisplayName", () => {
  const displayById = buildDisplayById(entries);

  it("fills when the current name is empty", () => {
    expect(
      resolveAutoDisplayName({
        currentName: "",
        prevModel: "",
        nextModel: "kimi-k2.6",
        displayById,
      }),
    ).toBe("Kimi K2.6");
  });

  it("overwrites when the current name equals the previous model's display", () => {
    expect(
      resolveAutoDisplayName({
        currentName: "Claude Sonnet 4.5",
        prevModel: "claude-sonnet-4-5",
        nextModel: "kimi-k2.6",
        displayById,
      }),
    ).toBe("Kimi K2.6");
  });

  it("keeps a manually edited name", () => {
    expect(
      resolveAutoDisplayName({
        currentName: "我的主力模型",
        prevModel: "claude-sonnet-4-5",
        nextModel: "kimi-k2.6",
        displayById,
      }),
    ).toBeNull();
  });

  it("returns null when the next model has no display", () => {
    expect(
      resolveAutoDisplayName({
        currentName: "",
        prevModel: "",
        nextModel: "no-display",
        displayById,
      }),
    ).toBeNull();
  });

  it("matches models carrying the [1m] suffix by their stripped id", () => {
    expect(
      resolveAutoDisplayName({
        currentName: "",
        prevModel: "",
        nextModel: "kimi-k2.6[1m]",
        displayById,
      }),
    ).toBe("Kimi K2.6");
    expect(
      resolveAutoDisplayName({
        currentName: "Claude Sonnet 4.5",
        prevModel: "claude-sonnet-4-5[1m]",
        nextModel: "kimi-k2.6[1m]",
        displayById,
      }),
    ).toBe("Kimi K2.6");
  });

  it("ignores whitespace-only current name (treats as empty)", () => {
    expect(
      resolveAutoDisplayName({
        currentName: "   ",
        prevModel: "",
        nextModel: "kimi-k2.6",
        displayById,
      }),
    ).toBe("Kimi K2.6");
  });
});

describe("backfillEmptyDisplayNames", () => {
  const displayById = buildDisplayById(entries);

  it("fills only roles whose name is empty and model has a display", () => {
    const result = backfillEmptyDisplayNames(
      { haiku: "kimi-k2.6[1m]", sonnet: "claude-sonnet-4-5", opus: "no-display" },
      { haiku: "", sonnet: "手动名字", opus: "" },
      displayById,
    );
    expect(result).toEqual({ haiku: "Kimi K2.6" });
  });

  it("returns an empty object when nothing to fill", () => {
    const result = backfillEmptyDisplayNames(
      { haiku: "unknown-model", sonnet: "", opus: "" },
      { haiku: "", sonnet: "", opus: "" },
      displayById,
    );
    expect(result).toEqual({});
  });
});

describe("parseCachedModelEntries", () => {
  it("parses the new {models:[{id,display}]} format", () => {
    const raw = JSON.stringify({
      base_url: "https://gw.example.com",
      models: [{ id: "kimi-k2.6", display: "Kimi K2.6" }],
      fetched_at: "2026-09-03T00:00:00Z",
    });
    expect(parseCachedModelEntries(raw)).toEqual([{ id: "kimi-k2.6", display: "Kimi K2.6" }]);
  });

  it("upgrades the legacy {model_ids: string[]} format", () => {
    const raw = JSON.stringify({
      base_url: "https://gw.example.com",
      model_ids: ["kimi-k2.6", "claude-sonnet-4-5"],
      fetched_at: "2026-09-03T00:00:00Z",
    });
    expect(parseCachedModelEntries(raw)).toEqual([
      { id: "kimi-k2.6" },
      { id: "claude-sonnet-4-5" },
    ]);
  });

  it("returns empty array for missing or malformed payloads", () => {
    expect(parseCachedModelEntries(null)).toEqual([]);
    expect(parseCachedModelEntries("not json")).toEqual([]);
    expect(parseCachedModelEntries("{}")).toEqual([]);
    expect(parseCachedModelEntries(JSON.stringify({ models: "nope" }))).toEqual([]);
  });
});
