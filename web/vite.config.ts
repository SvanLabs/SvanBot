import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// `npm run dev` proxies the API to the running dashboard (SVANBOT_WEB_PORT, default 5000);
// SVANBOT_API overrides the whole target.
const api = process.env.SVANBOT_API ?? `http://127.0.0.1:${process.env.SVANBOT_WEB_PORT ?? 5000}`;

export default defineConfig({
  plugins: [react()],
  server: { proxy: { '/api': { target: api, ws: true } } },
});
