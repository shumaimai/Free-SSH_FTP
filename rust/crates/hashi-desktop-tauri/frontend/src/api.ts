// 画面とRust側の境界。本番はTauriのcommand/eventだけを使い、試験用(VITE_MOCK)は偽の実装へ差し替える。
import type { Appearance } from './appearance';
import type {
  AiConnectArgs,
  AiConnectResult,
  AiServiceInfo,
  AiStatus,
  ApprovalRequest,
  ShareMode,
  Challenge,
  ChallengeAnswer,
  DataSink,
  Profile,
  SessionInfo,
  SessionStateEvent,
} from './types';

export interface Api {
  profilesList(): Promise<{ profiles: Profile[]; error: string | null }>;
  profileSave(index: number | null, profile: Profile): Promise<Profile[]>;
  profileDelete(index: number): Promise<Profile[]>;
  pickKeyFile(): Promise<string | null>;
  appearanceGet(): Promise<Appearance | null>;
  appearanceSet(appearance: Appearance): Promise<void>;
  challengeReply(challengeId: string, answer: ChallengeAnswer): Promise<void>;
  openLocal(cols: number, rows: number, sink: DataSink): Promise<SessionInfo>;
  connect(index: number, cols: number, rows: number, sink: DataSink): Promise<SessionInfo>;
  input(id: string, data: string): Promise<void>;
  resize(id: string, cols: number, rows: number): Promise<void>;
  close(id: string): Promise<void>;
  aiServices(): Promise<AiServiceInfo[]>;
  aiProtocols(): Promise<[string, string][]>;
  aiOpenKeyPage(service: string): Promise<void>;
  aiStatus(): Promise<AiStatus>;
  aiHasKey(service: string, base: string): Promise<boolean>;
  aiForget(service: string, base: string): Promise<void>;
  aiConnect(args: AiConnectArgs): Promise<AiConnectResult>;
  aiSwitchModel(model: string): Promise<void>;
  aiClear(): Promise<void>;
  aiStop(): Promise<void>;
  aiSend(question: string, targets: string[], mode: ShareMode): Promise<{ preview: string | null }>;
  aiConfirm(context: string): Promise<void>;
  aiCancelPreview(): Promise<void>;
  approvalReply(id: string, allow: boolean): Promise<void>;
  onAiDelta(handler: (text: string) => void): Promise<() => void>;
  onAiDone(handler: (event: { error: string | null; text: string }) => void): Promise<() => void>;
  onAiRestored(handler: () => void): Promise<() => void>;
  onApproval(handler: (request: ApprovalRequest) => void): Promise<() => void>;
  onApprovalClosed(handler: (id: string) => void): Promise<() => void>;
  onChallenge(handler: (challenge: Challenge) => void): Promise<() => void>;
  onSessionState(handler: (event: SessionStateEvent) => void): Promise<() => void>;
}

async function tauriApi(): Promise<Api> {
  const { invoke, Channel } = await import('@tauri-apps/api/core');
  const { listen } = await import('@tauri-apps/api/event');
  const channelFor = (sink: DataSink) => {
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (data) => sink(new Uint8Array(data));
    return channel;
  };
  return {
    profilesList: () => invoke('profiles_list'),
    profileSave: (index, profile) => invoke('profile_save', { index, profile }),
    profileDelete: (index) => invoke('profile_delete', { index }),
    pickKeyFile: () => invoke('pick_key_file'),
    appearanceGet: () => invoke('appearance_get'),
    appearanceSet: (appearance) => invoke('appearance_set', { appearance }),
    challengeReply: (challengeId, answer) => invoke('challenge_reply', { challengeId, answer }),
    openLocal: (cols, rows, sink) =>
      invoke('session_open_local', { cols, rows, onData: channelFor(sink) }),
    connect: (index, cols, rows, sink) =>
      invoke('session_connect', { index, cols, rows, onData: channelFor(sink) }),
    input: (id, data) => invoke('session_input', { id, data }),
    resize: (id, cols, rows) => invoke('session_resize', { id, cols, rows }),
    close: (id) => invoke('session_close', { id }),
    aiServices: () => invoke('ai_services'),
    aiProtocols: () => invoke('ai_protocols'),
    aiOpenKeyPage: (service) => invoke('ai_open_key_page', { service }),
    aiStatus: () => invoke('ai_status'),
    aiHasKey: (service, base) => invoke('ai_has_key', { service, base }),
    aiForget: (service, base) => invoke('ai_forget', { service, base }),
    aiConnect: (args) => invoke('ai_connect', { args }),
    aiSwitchModel: (model) => invoke('ai_switch_model', { model }),
    aiClear: () => invoke('ai_clear'),
    aiStop: () => invoke('ai_stop'),
    aiSend: (question, targets, mode) => invoke('ai_send', { question, targets, mode }),
    aiConfirm: (context) => invoke('ai_confirm', { context }),
    aiCancelPreview: () => invoke('ai_cancel_preview'),
    approvalReply: (id, allow) => invoke('approval_reply', { id, allow }),
    onAiDelta: (handler) => listen<string>('ai-delta', (e) => handler(e.payload)),
    onAiDone: (handler) => listen<{ error: string | null; text: string }>('ai-done', (e) => handler(e.payload)),
    onAiRestored: (handler) => listen('ai-restored', () => handler()),
    onApproval: (handler) => listen<ApprovalRequest>('approval', (e) => handler(e.payload)),
    onApprovalClosed: (handler) => listen<string>('approval-closed', (e) => handler(e.payload)),
    onChallenge: (handler) => listen<Challenge>('challenge', (e) => handler(e.payload)),
    onSessionState: (handler) =>
      listen<SessionStateEvent>('session-state', (e) => handler(e.payload)),
  };
}

export async function loadApi(): Promise<Api> {
  if (import.meta.env.VITE_MOCK) {
    return (await import('./mock')).mockApi();
  }
  return tauriApi();
}
