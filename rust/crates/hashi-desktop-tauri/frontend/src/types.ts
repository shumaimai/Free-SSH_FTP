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
