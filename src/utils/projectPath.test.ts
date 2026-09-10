import { describe, it, expect } from "vitest";
import { basename, isSubPath, findBestProjectPath } from "./projectPath";

describe("projectPath.ts", () => {
  it("extracts basename correctly for POSIX and Windows paths", () => {
    expect(basename("/Users/test/codes/Claudia")).toBe("Claudia");
    expect(basename("/Users/test/codes/Claudia/")).toBe("Claudia");
    expect(basename("C:\\Users\\test\\codes\\Claudia")).toBe("Claudia");
    expect(basename("C:\\Users\\test\\codes\\Claudia\\")).toBe("Claudia");
    expect(basename("Claudia")).toBe("Claudia");
    expect(basename("")).toBe("");
  });

  it("checks subpaths correctly", () => {
    expect(isSubPath("/Users/test/codes/Claudia/src", "/Users/test/codes/Claudia")).toBe(true);
    expect(isSubPath("C:\\Users\\test\\codes\\Claudia\\src", "C:\\Users\\test\\codes\\Claudia")).toBe(true);
    expect(isSubPath("/Users/test/other", "/Users/test/codes/Claudia")).toBe(false);
  });

  it("finds best matching project path", () => {
    const list = ["/Users/test/codes", "/Users/test/codes/Claudia"];
    expect(findBestProjectPath("/Users/test/codes/Claudia/src/App.vue", list)).toBe("/Users/test/codes/Claudia");
  });
});
