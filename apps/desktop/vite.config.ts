import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // The deterministic demo fixtures live in the repository-level `demo/`.
    fs: { allow: ["..", "../.."] },
  },
});
