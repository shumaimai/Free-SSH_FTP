// Rust側（hashi-storage::config::Profile ほか）と同じ形。変えるときは両方を直す。
export interface Profile {
  name: string;
  host: string;
  port: number;
  username: string;
  auth_method: 'key' | 'password' | 'agent';
  key_path: string;
  initial_path: string;
  proxy_jump: string;
  agent_forwarding: boolean;
  save_secrets: boolean;
  sudo_same_as_password: boolean;
  tags: string[];
  color: string;
  last_connected: number;
}

export const emptyProfile = (): Profile => ({
  name: '',
  host: '',
  port: 22,
  username: '',
  auth_method: 'key',
  key_path: '',
  initial_path: '',
  proxy_jump: '',
  agent_forwarding: false,
  save_secrets: true,
  sudo_same_as_password: true,
  tags: [],
  color: '',
  last_connected: 0,
});

export interface HostKey {
  key_type: string;
  fingerprint: string;
}

export interface Challenge {
  challenge_id: string;
  scope: { stamp: { terminal_id: string }; host: string; port: number; username: string; hop: number | null; kind: string };
  challenge:
    | { type: 'host_key'; key: HostKey; old: HostKey | null }
    | { type: 'secret'; prompt: string };
}

export type ChallengeAnswer =
  | { type: 'trust'; confirmation: string }
  | { type: 'secret'; value: string }
  | { type: 'cancel' };

export type SessionState = 'connecting' | 'connected' | 'closed' | 'error';

export interface SessionStateEvent {
  id: string;
  state: SessionState;
  message: string;
}

export interface SessionInfo {
  id: string;
  label: string;
}

export type DataSink = (bytes: Uint8Array) => void;

// ---- AI ----
export interface AiServiceInfo {
  id: string;
  label: string;
  needsKey: boolean;
  hasKeyPage: boolean;
  blurb: string;
  defaultBase: string;
}

export interface AiStatus {
  connected: boolean;
  service: string | null;
  model: string | null;
  models: string[];
  saved: { service?: string; base?: string; model?: string; protocol?: string; use_tools?: boolean } | null;
}

/** 登録したAIモデル1件。キーは含まない。 */
export interface AiModelEntry {
  id: string;
  name: string;
  service: string;
  base: string;
  /** 空なら使うときに一覧から選ぶ。 */
  model: string;
  protocol: string;
  useTools: boolean;
  resolved?: string;
}
export interface AiModelView extends AiModelEntry {
  label: string;
  /** キーが要らない、または保存済み。 */
  hasKey: boolean;
}
export interface AiModelsList {
  entries: AiModelView[];
  active: string | null;
  warning: string | null;
}
export interface AiCheckResult {
  ok: boolean;
  message: string;
  model: string | null;
  models: string[];
}
export function emptyAiModel(): AiModelEntry {
  return { id: '', name: '', service: 'openai', base: '', model: '', protocol: 'auto', useTools: true };
}

export type ShareMode = 'consult' | 'confirm' | 'auto';

export interface ApprovalRequest {
  id: string;
  actor: string;
  action: 'send_input' | 'send_keys' | 'run_command' | 'interrupt';
  label: string;
  generation: number;
  cwd: string | null;
  text: string;
}

// ---- ファイル ----
export interface FileEntry {
  name: string;
  path: string;
  directory: boolean;
  symlink: boolean;
  size: number;
  permissions: number | null;
}

/** 同期ブラウズの追随指示。絶対パスは写さず、相対移動だけがRust側で計算される。 */
export type Follow =
  | { kind: 'remote'; path: string }
  | { kind: 'local'; path: string }
  | { kind: 'failed'; message: string };

export interface Listing {
  path: string;
  entries: FileEntry[];
  follow: Follow | null;
}

export interface LocalListing {
  path: string;
  parent: string | null;
  entries: FileEntry[];
  follow: Follow | null;
}

export type JobPhase = 'queued' | 'running' | 'done' | 'failed' | 'cancelled';

export interface FileJob {
  id: string;
  sessionId: string;
  label: string;
  phase: JobPhase;
  done: number;
  total: number;
  message: string;
}

export type ExistingPolicy = 'refuse' | 'overwrite' | 'resume' | 'skip';
export type TransferMode = 'atomic' | 'in_place';

// ---- スニペット ----
export interface Snippet {
  name: string;
  body: string;
  send_enter: boolean;
}

// ---- 鍵生成・ポート転送 ----
export interface KeygenDefaults {
  ed25519: string;
  ecdsa: string;
  rsa: string;
  ecdsaBits: number[];
  rsaBits: number[];
}

export interface KeygenArgs {
  kind: 'ed25519' | 'ecdsa' | 'rsa';
  bits: number;
  passphrase: string;
  comment: string;
  path: string;
  overwrite: boolean;
  registerSession: string | null;
}

export interface KeygenResult {
  path: string;
  publicLine: string;
  registered: boolean | null;
  warning: string | null;
}

export interface ForwardInfo {
  id: string;
  label: string;
}

// ---- 設定・更新 ----
export interface AppSettings {
  updateCheck: boolean;
  sessionLog: boolean;
  sessionLogDir: string;
}

export interface UpdateInfo {
  tag: string;
  url: string;
}

export interface ExternalStatus {
  shared: number;
  mode: 'consult' | 'confirm' | 'auto';
  seconds: number;
  active: number;
  mcpRunning: boolean;
  mcpStarting: boolean;
  mcpConfig: string;
}

export interface UiPrefs {
  remoteHidden: boolean;
  localHidden: boolean;
  twoPane: boolean;
  dockOpen: boolean;
  dockTab: 'files' | 'ai';
}

export interface RegisterStatus {
  helperFound: boolean;
  helperPath: string;
  desktopConfigPath: string | null;
  desktopState: 'registered' | 'notRegistered' | 'otherPath' | 'unreadable' | 'noConfig';
  codeCommand: string;
  manualConfig: string;
  backup: string | null;
}

export interface PermissionStatus {
  enabled: boolean;
  busy: boolean;
  note: string | null;
}

/** エディタに開いた文書。保存先はRust側が持ち、画面は表示と編集内容だけを扱う。 */
export interface OpenedDoc {
  id: string;
  name: string;
  path: string;
  sessionId: string | null;
  kind: 'text' | 'hex';
  text: string | null;
  /** HEXのときの全バイト（base64）。 */
  hex: string | null;
  encoding: string;
  newline: string;
  size: number;
}

export interface SavedDoc {
  name: string;
  path: string;
  sessionId: string | null;
  size: number;
}

/** 旧版から取り込む候補の1件。秘密の値は画面へ来ない（有無だけ）。 */
export interface ImportProfile {
  index: number;
  label: string;
  id: string;
  auth_method: Profile['auth_method'];
  proxy_jump: boolean;
  exists: boolean;
  password: boolean;
  passphrase: boolean;
}

export interface ImportPreview {
  token: string;
  source: 'legacy' | 'bundle';
  needs_passphrase: boolean;
  unlocked: boolean;
  profiles: ImportProfile[];
  hosts_new: number;
  hosts_same: number;
  hosts_conflict: number;
  snippets_new: number | null;
  notes: string[];
}

export interface ImportOptions {
  token: string;
  selected: number[];
  overwrite: boolean;
  known_hosts: boolean;
  snippets: boolean;
  secrets: boolean;
}

export interface ImportResult {
  added: number;
  updated: number;
  skipped: number;
  hosts_added: number;
  host_conflicts: string[];
  snippets_added: number;
  secrets_saved: number;
  secrets_failed: number;
  notes: string[];
}

/** 接続先に保存されている秘密の種類（値は返らない）。 */
export interface SecretsStatus {
  saved: string[];
  backend: string;
}
