import type { Terminal } from "@xterm/xterm";
import { WebglAddon } from "@xterm/addon-webgl";

/** Render with WebGL when the webview allows it; xterm keeps its DOM renderer otherwise. */
export function loadWebglRenderer(term: Terminal) {
  try {
    const webgl = new WebglAddon();
    webgl.onContextLoss(() => webgl.dispose());
    term.loadAddon(webgl);
  } catch (error) {
    console.warn("WebGL terminal renderer unavailable, using the DOM renderer", error);
  }
}
