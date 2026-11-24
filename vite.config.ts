import { defineConfig } from 'vite';
import preact from '@preact/preset-vite';

export default defineConfig({
  plugins: [preact()],
  // Tailwind runs as a PostCSS plugin here; see postcss.config.mjs for why.
  css: {
    transformer: 'postcss',
  },
  server: {
    host: '0.0.0.0',
    port: 3000,
    strictPort: true,
    proxy: {
      '/api': 'http://api:3001',
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    rollupOptions: {
      // Two entries: the island the comparison pages load, and the dashboard SPA the admin
      // still uses. They are separate on purpose — the island is 8 KB and every page that has
      // a picker pays for it, so the dashboard's charts must not be in the same file.
      input: {
        island: 'src/island.tsx',
        main: 'index.html',
      },
    },
    // The Rust side reads this to learn the hashed asset names: a template cannot hard-code
    // `index-Ccu-D8U5.css`.
    manifest: true,
  },
});
