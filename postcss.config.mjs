// Tailwind 4 via PostCSS rather than @tailwindcss/vite.
//
// Vite 8 bundles CSS with Lightning CSS, which inlines `@import` and minifies before the
// Tailwind Vite plugin's transform runs — the result was a stylesheet containing the raw
// `@theme` block and zero generated utilities. The PostCSS plugin runs inside the CSS
// pipeline, so it wins regardless of the bundler.
//
// Note: no autoprefixer. Tailwind 4 handles vendor prefixes itself.
export default {
  plugins: {
    '@tailwindcss/postcss': {},
  },
};
