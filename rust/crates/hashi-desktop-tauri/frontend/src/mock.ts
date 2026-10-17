// 画面の確認・試験用の偽のバックエンド。VITE_MOCK付きのビルドにだけ入り、製品には入らない。
import type { Api } from './api';
import type {
  AppSettings,
  ForwardInfo,
  Snippet,
  FileEntry,
  FileJob,
  AiServiceInfo,
  AiStatus,
  ApprovalRequest,
  Challenge,
  ChallengeAnswer,
  DataSink,
  Profile,
  SessionInfo,
  SessionStateEvent,
  OpenedDoc,
  ImportPreview,
  AiModelEntry,
  AiModelsList,
} from './types';
import { emptyProfile } from './types';

function mockImport(source: 'legacy' | 'bundle', unlocked: boolean): ImportPreview {
  const secret = source === 'legacy' || unlocked;
  return {
    token: 'mock-token',
    source,
    needs_passphrase: source === 'bundle',
    unlocked,
    profiles: [
      { index: 0, label: '本番サーバー', id: 'deploy@prod.example.com:22', auth_method: 'key', proxy_jump: false, exists: false, password: false, passphrase: secret },
      { index: 1, label: '開発VPS', id: 'dev@192.0.2.10:2222', auth_method: 'password', proxy_jump: true, exists: false, password: secret, passphrase: false },
      { index: 2, label: '自宅NAS', id: 'admin@nas.local:22', auth_method: 'password', proxy_jump: false, exists: true, password: secret, passphrase: false },
    ],
    hosts_new: 2,
    hosts_same: 1,
    hosts_conflict: 1,
    snippets_new: source === 'legacy' ? 3 : null,
    notes: source === 'legacy' ? ['sudoパスワードはこのアプリが使わないため取り込みません'] : [],
  };
}

function mockDoc(sessionId: string | null, path: string): OpenedDoc {
  const name = path.split('/').pop() ?? path;
  const id = crypto.randomUUID();
  if (name.endsWith('.bin') || name.endsWith('.dat')) {
    const bytes = new Uint8Array(1500).map((_, i) => (i * 7 + (i >> 4)) & 0xff);
    let bin = '';
    bytes.forEach((b) => (bin += String.fromCharCode(b)));
    return { id, name, path, sessionId, kind: 'hex', text: null, hex: btoa(bin), encoding: 'UTF-8', newline: 'LF', size: bytes.length };
  }
  const text = `# ${name}\nserver {\n    listen 80;\n    server_name example.com;\n    # 日本語のコメント\n    location / {\n        proxy_pass http://127.0.0.1:8080;\n    }\n}\n`;
  return { id, name, path, sessionId, kind: 'text', text, hex: null, encoding: 'UTF-8', newline: 'LF', size: text.length };
}

const localTree: Record<string, FileEntry[]> = {
  '/home/me': [
    { name: 'Documents', path: '/home/me/Documents', directory: true, symlink: false, size: 0, permissions: null },
    { name: 'Downloads', path: '/home/me/Downloads', directory: true, symlink: false, size: 0, permissions: null },
    { name: 'notes.txt', path: '/home/me/notes.txt', directory: false, symlink: false, size: 2048, permissions: null },
    { name: 'photo.png', path: '/home/me/photo.png', directory: false, symlink: false, size: 1843200, permissions: null },
    { name: 'report.xlsx', path: '/home/me/report.xlsx', directory: false, symlink: false, size: 52311, permissions: null },
  ],
};

export function mockApi(): Api {
  let localOpens = 0;
  let permEnabled = false;
  let mcpOn = false;
  let registered: 'registered' | 'notRegistered' = 'notRegistered';
  const registerStatus = (backup: string | null = null) => ({
    helperFound: true,
    helperPath: 'C:\\Hashi\\HashiMCP-RustPreview.exe',
    desktopConfigPath: 'C:\\Users\\me\\AppData\\Roaming\\Claude\\claude_desktop_config.json',
    desktopState: registered,
    codeCommand: 'claude mcp add --scope user hashi-rust-preview -- "C:\\Hashi\\HashiMCP-RustPreview.exe"',
    manualConfig: JSON.stringify({ mcpServers: { 'hashi-rust-preview': { command: 'C:\\Hashi\\HashiMCP-RustPreview.exe', args: [] } } }, null, 2),
    backup,
  });
  let ext: { shared: number; mode: 'consult' | 'confirm' | 'auto' } = { shared: 0, mode: 'consult' };
  const extStatus = () => ({
    shared: ext.shared,
    mode: ext.mode,
    seconds: null,
    active: 0,
    mcpRunning: mcpOn,
    mcpStarting: false,
    mcpConfig: mcpOn
      ? JSON.stringify({ mcpServers: { 'hashi-rust-preview': { type: 'stdio', command: 'C:\\Hashi\\HashiMCP-RustPreview.exe', args: [] } } }, null, 2)
      : '',
  });
  let stored: import('./appearance').Appearance | null = null;
  let storedPrefs: import('./types').UiPrefs | null = null;
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
  const services: AiServiceInfo[] = [
    { id: 'openai', label: 'OpenAI', needsKey: true, hasKeyPage: true, blurb: 'ChatGPTのAPI。利用料はOpenAIの料金に従います。', defaultBase: 'https://api.openai.com/v1' },
    { id: 'anthropic', label: 'Claude', needsKey: true, hasKeyPage: true, blurb: 'AnthropicのAPI。利用料はAnthropicの料金に従います。', defaultBase: 'https://api.anthropic.com/v1' },
    { id: 'local', label: '自分のPC', needsKey: false, hasKeyPage: false, blurb: 'LM StudioやOllamaなど、このPCで動くAI。キーは要りません。', defaultBase: 'http://127.0.0.1:1234/v1' },
    { id: 'custom', label: 'その他', needsKey: true, hasKeyPage: false, blurb: 'OpenAI互換のAPIを出している他社サービス。接続先URLを入力してください。', defaultBase: '' },
  ];
  const ai: AiStatus = { connected: false, service: null, model: null, models: [], saved: null };
  let models: AiModelEntry[] = [];
  let activeModel: string | null = null;
  const savedKeys = new Set<string>(['anthropic|']);
  const modelChanged = new Set<() => void>();
  const keyOf = (e: AiModelEntry) => `${e.service}|${e.base}`;
  const labelOf = (e: AiModelEntry) =>
    e.name || `${{ openai: 'OpenAI', anthropic: 'Claude', local: '自分のPC', custom: 'その他' }[e.service] ?? 'AI'} / ${e.model || 'モデルは自動'}`;
  const modelsList = (): AiModelsList => ({
    entries: models.map((e) => ({ ...e, label: labelOf(e), hasKey: e.service === 'local' || savedKeys.has(keyOf(e)) })),
    active: activeModel,
    warning: null,
  });
  const useModel = async (id: string) => {
    const e = models.find((m) => m.id === id);
    if (!e) throw new Error('登録が見つかりません');
    if (e.service !== 'local' && !savedKeys.has(keyOf(e))) {
      throw new Error('このモデルのAPIキーが保存されていません。AIモデルの管理でキーを入力してください');
    }
    activeModel = id;
    Object.assign(ai, { connected: true, service: e.service, model: e.model || 'gpt-5', models: [] });
    modelChanged.forEach((h) => h());
    return { model: ai.model ?? '' };
  };
  const deltaHandlers = new Set<(t: string) => void>();
  const doneHandlers = new Set<(e: { error: string | null; text: string }) => void>();
  const approvalHandlers = new Set<(r: ApprovalRequest) => void>();
  let pendingQuestion = '';
  const stream = (question: string) => {
    const answer = `「${question}」ですね。\n\n端末の出力を確認しました。次のコマンドで状況を調べられます。\n\n\`\`\`bash\ndf -h\n\`\`\`\n\n容量が逼迫していれば、不要なログを削除してください。`;
    const chunks = answer.match(/[\s\S]{1,6}/g) ?? [];
    let i = 0;
    const timer = setInterval(() => {
      if (i < chunks.length) deltaHandlers.forEach((h) => h(chunks[i++]));
      else {
        clearInterval(timer);
        doneHandlers.forEach((h) => h({ error: null, text: answer }));
      }
    }, 40);
  };
  const tree: Record<string, FileEntry[]> = {
    '/home/tester': [
      { name: 'docs', path: '/home/tester/docs', directory: true, symlink: false, size: 4096, permissions: 0o755 },
      { name: 'projects', path: '/home/tester/projects', directory: true, symlink: false, size: 4096, permissions: 0o755 },
      { name: 'archive.tar.gz', path: '/home/tester/archive.tar.gz', directory: false, symlink: false, size: 5_452_595, permissions: 0o644 },
      { name: 'readme.txt', path: '/home/tester/readme.txt', directory: false, symlink: false, size: 412, permissions: 0o644 },
      { name: 'メモ.md', path: '/home/tester/メモ.md', directory: false, symlink: false, size: 1536, permissions: 0o644 },
    ],
    '/home/tester/docs': [
      { name: 'manual.pdf', path: '/home/tester/docs/manual.pdf', directory: false, symlink: false, size: 2_097_152, permissions: 0o644 },
    ],
  };
  const jobHandlers = new Set<(j: FileJob) => void>();
  const changedHandlers = new Set<(id: string) => void>();
  let jobs: FileJob[] = [];
  const addJob = (sessionId: string, label: string) => {
    const job: FileJob = { id: `job-${jobs.length + 1}`, sessionId, label, phase: 'running', done: 0, total: 100, message: '' };
    jobs = [...jobs, job];
    jobHandlers.forEach((h) => h({ ...job }));
    let n = 0;
    const timer = setInterval(() => {
      n += 25;
      job.done = Math.min(100, n);
      if (n >= 100) {
        clearInterval(timer);
        job.phase = 'done';
        job.message = '1ファイル / 0フォルダ / 0スキップ';
        changedHandlers.forEach((h) => h(sessionId));
      }
      jobHandlers.forEach((h) => h({ ...job }));
    }, 300);
  };
  let snippets: Snippet[] = [
    { name: 'ディスク使用量', body: 'df -h', send_enter: true },
    { name: 'ログを追う', body: 'tail -f {{file}}', send_enter: true },
    { name: 'サービス再起動', body: 'sudo systemctl restart {{service}}', send_enter: false },
  ];
  let forwards: ForwardInfo[] = [];
  let appSettings: AppSettings = { updateCheck: true, sessionLog: false, sessionLogDir: '' };
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
    uiPrefsGet: async () => storedPrefs,
    uiPrefsSet: async (prefs) => {
      storedPrefs = prefs;
    },
    appearanceSet: async (a) => {
      stored = a;
    },
    aiServices: async () => services,
    aiProtocols: async () => [['auto', '自動'], ['chat', 'Chat Completions'], ['responses', 'Responses'], ['messages', 'Anthropic Messages']],
    aiOpenKeyPage: async () => {},
    aiStatus: async () => ({ ...ai }),
    aiHasKey: async (service) => service === 'anthropic',
    aiForget: async () => {},
    aiModelsList: async () => modelsList(),
    aiModelSave: async (entry, key) => {
      if (!entry.id) entry = { ...entry, id: crypto.randomUUID() };
      if (key) savedKeys.add(keyOf(entry));
      const i = models.findIndex((m) => m.id === entry.id);
      if (i >= 0) models[i] = entry;
      else models.push(entry);
      if (!activeModel) await useModel(entry.id);
      modelChanged.forEach((h) => h());
      return modelsList();
    },
    aiModelDelete: async (id) => {
      models = models.filter((m) => m.id !== id);
      if (activeModel === id) {
        activeModel = null;
        Object.assign(ai, { connected: false, service: null, model: null });
      }
      modelChanged.forEach((h) => h());
      return modelsList();
    },
    aiModelUse: async (id) => useModel(id),
    aiModelCheck: async (entry, key) => {
      await new Promise((r) => setTimeout(r, 400));
      if (key === 'bad') throw new Error('AI接続エラー HTTP 401: この接続先のAPIキーを確認してください');
      const list = ['gpt-5', 'gpt-5-mini', 'gpt-4.1'];
      if (!entry.model && key === 'nomodel') return { ok: false, message: '自動で選んだモデルで確認に失敗しました。別のモデル名を選んでください。', model: null, models: list };
      return { ok: true, message: '接続できました', model: entry.model || list[0], models: list };
    },
    aiModelCandidates: async () => ['gpt-5', 'gpt-5-mini', 'gpt-4.1', 'o3'],
    onAiModelsChanged: async (h) => {
      modelChanged.add(h);
      return () => modelChanged.delete(h);
    },
    aiClear: async () => {},
    aiStop: async () => {},
    aiSend: async (question, targets, _mode, attachments) => {
      if (attachments.length) question = `${question}\n(添付 ${attachments.length}件)`;
      pendingQuestion = question;
      if (targets.length > 0) {
        return { preview: JSON.stringify({ terminal_observations: [{ label: 'CMD', screen: '$ df -h\nFilesystem 98% /', trust: 'untrusted_observation' }] }, null, 2) };
      }
      stream(question);
      return { preview: null };
    },
    aiConfirm: async () => stream(pendingQuestion),
    aiCancelPreview: async () => {},
    approvalReply: async () => {},
    onAiDelta: async (h) => {
      deltaHandlers.add(h);
      return () => deltaHandlers.delete(h);
    },
    onAiDone: async (h) => {
      doneHandlers.add(h);
      return () => doneHandlers.delete(h);
    },
    onAiRestored: async () => () => {},
    onApproval: async (h) => {
      approvalHandlers.add(h);
      return () => approvalHandlers.delete(h);
    },
    onApprovalClosed: async () => () => {},
    filesList: async (_id, path) => {
      const p = !path || path === '.' ? '/home/tester' : path.replace(/\/\.\.$/, '') || '/';
      const dir = p.endsWith('/..') ? '/home/tester' : p;
      return { path: dir, entries: tree[dir] ?? [], follow: null };
    },
    filesDownloadHere: async (id, _p, name) => addJob(id, `受信 ${name}`),
    localList: async (path) => {
      const dir = !path ? '/home/me' : path;
      const files = localTree[dir] ?? [];
      return { path: dir, parent: dir === '/' ? null : dir.replace(/\/[^/]+$/, '') || '/', entries: files, follow: null };
    },
    localPickDir: async () => null,
    localRoots: async () => ['/'],
    localUpload: async (id, names) => {
      names.forEach((n) => addJob(id, `送信 ${n}`));
      return names.length;
    },
    syncGet: async () => false,
    syncSet: async () => {},
    onLocalChanged: async () => () => {},
    filesSearch: async (_id, _root, query) => (tree['/home/tester'] ?? []).filter((e) => e.name.includes(query)),
    filesJobs: async () => jobs,
    filesCancel: async () => {},
    filesClearJobs: async () => {
      jobs = jobs.filter((j) => j.phase === 'running');
    },
    filesUploadPick: async (id, _dir, folder) => addJob(id, folder ? '送信 projects' : '送信 report.xlsx'),
    filesUploadDropped: async (id) => {
      addJob(id, '送信 dropped.txt');
      return 1;
    },
    filesDownload: async (id, _p, name) => addJob(id, `受信 ${name}`),
    filesDelete: async (id, _p, name) => addJob(id, `削除 ${name}`),
    filesBookmarks: async () => ['/home/tester/docs', '/var/www'],
    filesBookmarkAdd: async () => ['/home/tester/docs', '/var/www'],
    onFileJob: async (h) => {
      jobHandlers.add(h);
      return () => jobHandlers.delete(h);
    },
    onFilesChanged: async (h) => {
      changedHandlers.add(h);
      return () => changedHandlers.delete(h);
    },
    onFileDrop: async () => () => {},
    onFilesState: async (h) => {
      setTimeout(() => h({ id: 'ssh-1', available: true, message: '' }), 100);
      return () => {};
    },
    externalStatus: async () => extStatus(),
    externalShare: async (mode, targets) => {
      ext = { shared: targets.length, mode };
      mcpOn = true;
      return extStatus();
    },
    externalStop: async () => {
      ext = { shared: 0, mode: 'consult' };
      mcpOn = false;
      return extStatus();
    },
    mcpRegisterStatus: async () => registerStatus(),
    mcpRegisterDesktop: async () => {
      registered = 'registered';
      return registerStatus('C:\\Users\\me\\AppData\\Roaming\\Claude\\claude_desktop_config.json.hashi-backup-1');
    },
    importLegacyInspect: async (legacyStopped) => {
      if (!legacyStopped) throw new Error('旧Python版をすべて終了したことの確認が必要です');
      return mockImport('legacy', true);
    },
    importBundlePick: async () => mockImport('bundle', false),
    importBundleUnlock: async (_token, passphrase) => {
      if (passphrase !== 'pass') throw new Error('パスフレーズが違うか、秘密情報が壊れています');
      return mockImport('bundle', true);
    },
    importCancel: async () => {},
    importApply: async (options) => [
      {
        added: options.selected.length,
        updated: 0,
        skipped: 0,
        hosts_added: options.known_hosts ? 2 : 0,
        host_conflicts: options.known_hosts ? ['nas.local:22'] : [],
        snippets_added: options.snippets ? 3 : 0,
        secrets_saved: options.secrets ? 2 : 0,
        secrets_failed: 0,
        notes: [],
      },
      profiles,
    ],
    secretsStatus: async () => ({ saved: ['password'], backend: 'Windows資格情報マネージャ' }),
    secretsForget: async () => {},
    permissionsStatus: async () => ({ enabled: permEnabled, busy: false, note: permEnabled ? '旧版の残存マーカー0件、復旧記録の確認先1件（未復元0件）。旧記録は変更していません。' : null }),
    permissionsEnable: async () => {
      permEnabled = true;
      return { enabled: true, busy: false, note: '旧版の残存マーカー0件、復旧記録の確認先1件（未復元0件）。旧記録は変更していません。' };
    },
    permissionsRecover: async () => '権限復旧: 0件復元しました',
    onSessionWarning: async () => () => {},
    editorOpenRemote: async (sessionId, path) => mockDoc(sessionId, path),
    editorOpenLocal: async () => mockDoc(null, localOpens++ % 2 === 0 ? '/home/me/nginx.conf' : '/home/me/world.dat'),
    editorSave: async (_id, text) => ({ name: 'saved', path: '/x/saved', sessionId: null, size: text?.length ?? 0 }),
    editorSaveAs: async () => ({ name: 'copy.txt', path: '/home/me/copy.txt', sessionId: null, size: 1 }),
    editorSetDirty: async () => {},
    editorClose: async () => {},
    appQuit: async () => {},
    onCloseRequested: async () => () => {},
    snippetsList: async () => snippets,
    snippetsSave: async (list) => {
      snippets = list;
      return snippets;
    },
    snippetVariables: async (body) => [...new Set([...body.matchAll(/\{\{\s*([^{}\s]+)\s*\}\}/g)].map((m) => m[1]))],
    snippetSend: async (id, index, values) => {
      const sn = snippets[index];
      write(id, sn.body.replace(/\{\{\s*([^{}\s]+)\s*\}\}/g, (_m, k) => values[k] ?? '') + (sn.send_enter ? '\r\n' : ''));
    },
    keygenDefaults: async () => ({
      ed25519: 'C:\\Users\\me\\.ssh\\id_ed25519',
      ecdsa: 'C:\\Users\\me\\.ssh\\id_ecdsa',
      rsa: 'C:\\Users\\me\\.ssh\\id_rsa',
      ecdsaBits: [256, 384, 521],
      rsaBits: [2048, 3072, 4096],
    }),
    keygenPathExists: async (path) => path.endsWith('id_rsa'),
    keygenPickPath: async () => 'C:\\Users\\me\\.ssh\\id_work',
    keygenGenerate: async (args) => ({
      path: args.path,
      publicLine: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJq8x0m3EXAMPLEKEYDATAxxxxxxxxxxxxxxxxxxxx hashi',
      registered: args.registerSession ? true : null,
      warning: null,
    }),
    forwardList: async () => forwards,
    forwardStart: async (_id, kind, bind, port, host, targetPort) => {
      forwards = [...forwards, { id: `f${forwards.length + 1}`, label: `${kind === 'D' ? 'SOCKS5' : kind === 'R' ? 'リモート' : 'ローカル'} ${bind}:${port}${kind === 'D' ? '' : ` → ${host}:${targetPort}`}` }];
      return forwards;
    },
    forwardStop: async (_id, forwardId) => {
      forwards = forwards.filter((f) => f.id !== forwardId);
      return forwards;
    },
    settingsGet: async () => appSettings,
    settingsSet: async (v) => {
      appSettings = v;
    },
    settingsPickLogDir: async () => 'D:\\logs',
    updateStatus: async () => null,
    updateOpenPage: async () => {},
    onUpdateAvailable: async () => () => {},
    onSessionLogError: async () => () => {},
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
