import { defineConfig } from 'vite'
import preact from '@preact/preset-vite'
import { viteSingleFile } from 'vite-plugin-singlefile'

// Builds the whole console into a single self-contained index.html (CSS + JS
// inlined) so the Rust binary can embed it with include_str! — no runtime
// assets, no extra Rust dependency.
export default defineConfig({
  plugins: [preact(), viteSingleFile()],
  build: {
    target: 'es2020',
    outDir: 'dist',
    emptyOutDir: true,
    assetsInlineLimit: 100_000_000,
    chunkSizeWarningLimit: 100_000_000,
  },
  // During `npm run dev`, proxy the info endpoint and websocket to a locally
  // running porticus so the console works against a real bridge in dev too.
  server: {
    proxy: {
      '/info': 'http://127.0.0.1:8081',
    },
  },
})
