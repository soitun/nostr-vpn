import { readFileSync } from 'node:fs';
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

type PackageJson = {
  version?: string;
};

const packageJson = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
) as PackageJson;
const buildTime = new Date().toISOString();

export default defineConfig({
  base: './',
  define: {
    'import.meta.env.VITE_APP_VERSION': JSON.stringify(packageJson.version ?? 'development'),
    'import.meta.env.VITE_BUILD_TIME': JSON.stringify(buildTime),
  },
  plugins: [svelte()],
  server: {
    host: '0.0.0.0',
    port: 4180,
  },
});
