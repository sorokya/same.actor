import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
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
  },
});
