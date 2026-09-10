import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  wrapMermaidBlocks,
  readMermaidTransform,
  setMermaidTransform,
  resetMermaidTransform,
  zoomMermaidBlock,
  setMermaidFullscreen,
  mountMermaidSvg,
  renderSingleMermaidBlock,
} from "./mermaid";

describe("mermaid utility functions", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("wrapMermaidBlocks transforms pre > code.language-mermaid into markdown-mermaid-block", () => {
    const container = document.createElement("div");
    container.innerHTML = `
      <pre class="hljs language-mermaid"><code class="language-mermaid">graph TD;\n  A-->B;</code></pre>
    `;
    document.body.appendChild(container);

    const blocks = wrapMermaidBlocks(container);
    expect(blocks).toHaveLength(1);

    const block = blocks[0];
    expect(block.classList.contains("markdown-mermaid-block")).toBe(true);
    expect(block.dataset.mermaidSource).toContain("graph TD;\n  A-->B;");

    const toolbar = block.querySelector(".markdown-mermaid-toolbar");
    expect(toolbar).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="copy"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="zoomIn"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="zoomOut"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="reset"]')).not.toBeNull();
    expect(toolbar?.querySelector('button[data-mermaid-action="fullscreen"]')).not.toBeNull();

    const canvas = block.querySelector(".markdown-mermaid-canvas");
    expect(canvas).not.toBeNull();
    const content = block.querySelector(".markdown-mermaid-content");
    expect(content).not.toBeNull();
    expect(content?.textContent).toContain("graph TD;\n  A-->B;");
  });

  it("pan and zoom transform functions correctly clamp and apply values", () => {
    const block = document.createElement("div");
    const content = document.createElement("div");
    content.className = "markdown-mermaid-content";
    block.appendChild(content);

    resetMermaidTransform(block);
    let transform = readMermaidTransform(block);
    expect(transform.scale).toBe(1);
    expect(transform.x).toBe(0);
    expect(transform.y).toBe(0);

    setMermaidTransform(block, { scale: 2, x: 50, y: -30 });
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(2);
    expect(transform.x).toBe(50);
    expect(transform.y).toBe(-30);
    expect(content.style.transform).toBe("translate(50px, -30px) scale(2)");

    // Zoom beyond max scale (4.0) clamps
    zoomMermaidBlock(block, 3);
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(4.0);

    // Zoom below min scale (0.3) clamps
    zoomMermaidBlock(block, 0.01);
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(0.3);

    // Reset restores scale 1 and (0,0)
    resetMermaidTransform(block);
    transform = readMermaidTransform(block);
    expect(transform.scale).toBe(1);
    expect(transform.x).toBe(0);
    expect(transform.y).toBe(0);
  });

  it("setMermaidFullscreen toggles fullscreen class and manages backdrop", () => {
    const block = document.createElement("div");
    block.className = "markdown-mermaid-block";
    const btn = document.createElement("button");
    btn.dataset.mermaidAction = "fullscreen";
    block.appendChild(btn);
    document.body.appendChild(block);

    setMermaidFullscreen(block, true);
    expect(block.classList.contains("markdown-mermaid-fullscreen")).toBe(true);
    expect(document.querySelector(".markdown-mermaid-fullscreen-backdrop")).not.toBeNull();

    setMermaidFullscreen(block, false);
    expect(block.classList.contains("markdown-mermaid-fullscreen")).toBe(false);
    expect(document.querySelector(".markdown-mermaid-fullscreen-backdrop")).toBeNull();
  });

  it("mountMermaidSvg extracts SVG styles to container for WKWebView compatibility", () => {
    const content = document.createElement("div");
    const svgMarkup = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><style>.node { fill: red; }</style><rect class="node" width="10" height="10"/></svg>`;

    mountMermaidSvg(content, svgMarkup);
    const styleEl = content.querySelector("style[data-mermaid-stylesheet]");
    expect(styleEl).not.toBeNull();
    expect(styleEl?.textContent).toContain(".node { fill: red; }");
    expect(content.querySelector("svg")).not.toBeNull();
  });

  it("renderSingleMermaidBlock handles rendering errors gracefully", async () => {
    const block = document.createElement("div");
    block.dataset.mermaidSource = "invalid mermaid syntax syntax error !!!";
    const content = document.createElement("div");
    content.className = "markdown-mermaid-content";
    block.appendChild(content);
    document.body.appendChild(block);

    await renderSingleMermaidBlock(block, "light");
    expect(block.dataset.mermaidError).toBe("true");
    expect(content.classList.contains("markdown-mermaid-error")).toBe(true);
    expect(content.textContent).toContain("invalid mermaid syntax");
  });
});
