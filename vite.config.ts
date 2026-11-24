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
      // One entry: the picker island. There is no single-page app any more — every page is
      // rendered by the server, and this is the only script the site loads.
      input: {
        island: 'src/island.tsx',
      },
    },
    // The Rust side reads this to learn the hashed asset names: a template cannot hard-code
    // `index-Ccu-D8U5.css`.
    manifest: true,
  },
});
