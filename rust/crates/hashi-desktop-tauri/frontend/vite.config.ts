import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// 本番はTauriが同梱する静的ファイルだけを読む。外部のURL・CDN・フォントは使わない。
export default defineConfig({
  base: './',
  plugins: [react()],
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false, target: 'es2022' },
  server: { port: 5173, strictPort: true },
});
