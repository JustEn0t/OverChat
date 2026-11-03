import { defineConfig } from "vite";
import { resolve } from "node:path";

// Two pages: settings window (index.html) and the chat overlay (overlay.html).
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: "es2022",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        overlay: resolve(__dirname, "overlay.html"),
      },
    },
  },
});
