import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      // HTTP API 代理到 Rust 后端
      "/api": {
        target: "http://localhost:3000",
        changeOrigin: true,
      },
      // WebSocket 实时流代理到 Rust 后端
      "/ws": {
        target: "http://localhost:3000",
        ws: true,
      },
    },
  },
});
