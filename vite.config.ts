import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port: 1432,
    strictPort: true,
    // Extracted official installers and generated QA fixtures are not app source.
    watch: { ignored: ["**/.qa/**", "**/delivery/**", "**/src-tauri/target/**"] },
  },
});
