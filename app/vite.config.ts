import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));

export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: {
    target: "safari15",
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: { index: here("./index.html"), notch: here("./notch.html") },
    },
  },
});
