import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const now = Math.floor(Date.now() / 1000);
const board = ['9c', '8h', '3h'];
const figures = {hands: 1240, priced_hands: 1240, net_chips: 2180, bb100: 8.8, confidence: 5.1};
const snapshot = {
  bots: [0, 1, 2].map(slot => ({
    slot, name: ['Demo Anchor', 'Demo Hunter', 'Demo Grinder'][slot], mode: 'playing',
    status: 'Playing demo hand', connected: true, last_error: null,
    table_id: null, hand_id: 'archive-preview', board, hole: ['As', 'Kd'],
    seats: Array.from({length: 6}, (_, seat) => ({
      seat, name: seat === 3 ? 'Demo bot' : 'Demo seat ' + (seat + 1),
      stack: 2000 - seat * 70, bet: seat < 2 ? 40 : 0, folded: seat === 4,
    })),
    hero_seat: 3, dealer_seat: 5, actor_seat: 1, pot: 300, big_blind: 20,
    street: 'flop', decision: null, version: '10.0.1', turn_started: null, turn: null, season: {},
    metrics: {...figures, all_time: figures, season: {scoped: false, number: null, id: null, started_at: null},
      p95_ms: 83, rejected: 0, decisions: 1700, series: Array.from({length: 20}, (_, n) => (
        {hand: n * 60, total: n * 100 + (n % 3) * 120, showdown: n * 70, other: n * 30}
      ))},
  })),
  training: {status: 'Archive preview', automatic: false, can_rollback: false,
    champion: {version: '10.0.1', name: 'Demo champion'}, progress: {hands: 0, target: 1000},
    experiments: [], opponent_profiles_tracked: 0},
  logs: [{id: 1, slot: null, ts: now, level: 'info', message: 'Archived v10 dashboard preview · sample data'}],
  updated: now,
  config: {buy_in: 2000, auto_rebuy: false, host: 'Preview only', port: 5555, configured_slots: 3},
};

export default defineConfig({
  server: { fs: { allow: [fileURLToPath(new URL('..', import.meta.url))] } },
  plugins: [react(), {
    name: 'archive-preview-data',
    configureServer(server) {
      server.middlewares.use('/api', (req, res) => {
        const path = req.url?.split('?')[0];
        if (path === '/events' && req.method === 'GET') {
          res.writeHead(200, {'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache'});
          res.write('event: state\ndata: ' + JSON.stringify(snapshot) + '\n\n');
          return;
        }
        let value: unknown;
        if (path === '/health') value = {public: false};
        else if (path === '/session' && req.method === 'POST') value = {ok: true};
        else if (req.method === 'GET' && path === '/state') value = snapshot;
        else if (req.method === 'GET' && path === '/leaderboard') value = {entries: [], season: null, stale: false};
        else {
          res.statusCode = req.method === 'GET' ? 404 : 405;
          value = {detail: 'Unavailable in the read-only archive preview'};
        }
        res.setHeader('Content-Type', 'application/json');
        res.end(JSON.stringify(value));
      });
    },
  }],
});
