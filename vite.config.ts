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
      // Two entries, and that is the whole front end: the picker (comparison pages only) and
      // the link booster (every public page). No single-page app.
      input: {
        island: 'src/island.tsx',
        boost: 'src/boost.ts',
        synergy: 'src/synergy.ts',
      },
    },
    // The Rust side reads this to learn the hashed asset names: a template cannot hard-code
    // `index-Ccu-D8U5.css`.
    manifest: true,
  },
});
