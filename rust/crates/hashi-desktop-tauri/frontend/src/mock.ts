// 画面の確認・試験用の偽のバックエンド。VITE_MOCK付きのビルドにだけ入り、製品には入らない。
import type { Api } from './api';
import type {
  Challenge,
  ChallengeAnswer,
  DataSink,
  Profile,
  SessionInfo,
  SessionStateEvent,
} from './types';
import { emptyProfile } from './types';

export function mockApi(): Api {
  let stored: import('./appearance').Appearance | null = null;
  let profiles: Profile[] = [
    { ...emptyProfile(), name: '本番サーバー', host: 'prod.example.com', username: 'deploy' },
    {
      ...emptyProfile(),
      name: '検証機',
      host: '192.168.1.20',
      username: 'tester',
      auth_method: 'password',
    },
    {
      ...emptyProfile(),
      host: 'build.internal',
      username: 'ci',
      proxy_jump: 'ops@bastion.example.com',
    },
  ];
  const challengeHandlers = new Set<(c: Challenge) => void>();
  const stateHandlers = new Set<(e: SessionStateEvent) => void>();
  const sinks = new Map<string, DataSink>();
  const encoder = new TextEncoder();
  const emitState = (e: SessionStateEvent) => stateHandlers.forEach((h) => h(e));
  const write = (id: string, text: string) => sinks.get(id)?.(encoder.encode(text));
  const prompt = (id: string) => write(id, '\x1b[32mtester@hashi\x1b[0m:\x1b[34m~\x1b[0m$ ');
  let next = 1;
  let pendingChallenge: { id: string; session: string } | null = null;

  const banner = (id: string) => {
    write(id, 'Hashi Preview (mock)\r\n日本語の表示: 橋 — ターミナルの確認\r\n');
    write(id, '\x1b[1mls\x1b[0m\r\ndocs  \x1b[34mprojects\x1b[0m  readme.txt  \x1b[31marchive.tar.gz\x1b[0m\r\n');
    prompt(id);
  };

  return {
    profilesList: async () => ({ profiles, error: null }),
    profileSave: async (index, profile) => {
      if (index === null) profiles = [...profiles, profile];
      else profiles = profiles.map((p, i) => (i === index ? profile : p));
      return profiles;
    },
    profileDelete: async (index) => {
      profiles = profiles.filter((_, i) => i !== index);
      return profiles;
    },
    appearanceGet: async () => stored,
    appearanceSet: async (a) => {
      stored = a;
    },
    pickKeyFile: async () => 'C:\\Users\\me\\.ssh\\id_ed25519',
    challengeReply: async (challengeId, answer: ChallengeAnswer) => {
      if (!pendingChallenge || pendingChallenge.id !== challengeId) return;
      const { session } = pendingChallenge;
      pendingChallenge = null;
      if (answer.type === 'cancel') {
        emitState({ id: session, state: 'error', message: '認証を取り消しました' });
        return;
      }
      emitState({ id: session, state: 'connected', message: '接続済み' });
      banner(session);
    },
    openLocal: async (_cols, _rows, sink): Promise<SessionInfo> => {
      const id = `local-${next++}`;
      sinks.set(id, sink);
      setTimeout(() => {
        emitState({ id, state: 'connected', message: 'ローカル端末' });
        banner(id);
      }, 50);
      return { id, label: 'CMD' };
    },
    connect: async (index, _cols, _rows, sink): Promise<SessionInfo> => {
      const id = `ssh-${next++}`;
      sinks.set(id, sink);
      const p = profiles[index];
      setTimeout(() => {
        emitState({ id, state: 'connecting', message: '接続中' });
        pendingChallenge = { id: `c-${id}`, session: id };
        const challenge: Challenge = {
          challenge_id: pendingChallenge.id,
          scope: { stamp: { terminal_id: id }, host: p.host, port: p.port, username: p.username, hop: null, kind: 'host_key' },
          challenge: {
            type: 'host_key',
            key: { key_type: 'ssh-ed25519', fingerprint: 'SHA256:P/HIFmTPkxrhg/r8XtF3JYNnNXwRZUgim69kKrFQmIc' },
            old: null,
          },
        };
        challengeHandlers.forEach((h) => h(challenge));
      }, 50);
      return { id, label: p.name || p.host };
    },
    input: async (id, data) => {
      // 打った文字をそのまま返し、Enterで次の行へ進める。
      write(id, data.replace(/\r/g, '\r\n'));
      if (data.includes('\r')) prompt(id);
    },
    resize: async () => {},
    close: async (id) => {
      sinks.delete(id);
      emitState({ id, state: 'closed', message: '閉じました' });
    },
    onChallenge: async (handler) => {
      challengeHandlers.add(handler);
      return () => challengeHandlers.delete(handler);
    },
    onSessionState: async (handler) => {
      stateHandlers.add(handler);
      return () => stateHandlers.delete(handler);
    },
  };
}
