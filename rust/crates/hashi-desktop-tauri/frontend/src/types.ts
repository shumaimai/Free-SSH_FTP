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

export interface AiConnectArgs {
  service: string;
  base: string;
  key: string;
  model: string;
  protocol: string;
  useTools: boolean;
  persist: boolean;
}

export interface AiConnectResult {
  kind: 'connected' | 'need_model';
  message: string;
  model: string | null;
  models: string[];
}

export type ShareMode = 'consult' | 'confirm' | 'auto';

export interface ApprovalRequest {
  id: string;
  actor: string;
  action: 'send_input' | 'run_command' | 'interrupt';
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

export interface Listing {
  path: string;
  entries: FileEntry[];
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
