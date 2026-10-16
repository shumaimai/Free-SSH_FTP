// 画面とRust側の境界。本番はTauriのcommand/eventだけを使い、試験用(VITE_MOCK)は偽の実装へ差し替える。
import type { Appearance } from './appearance';
import type {
  AppSettings,
  UpdateInfo,
  ForwardInfo,
  KeygenArgs,
  KeygenDefaults,
  KeygenResult,
  Snippet,
  ExistingPolicy,
  FileEntry,
  FileJob,
  Listing,
  TransferMode,
  AiModelEntry,
  AiModelsList,
  AiCheckResult,
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
  OpenedDoc,
  SavedDoc,
  PermissionStatus,
  ExternalStatus,
  LocalListing,
  ImportPreview,
  ImportOptions,
  ImportResult,
  SecretsStatus,
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
  aiModelsList(): Promise<AiModelsList>;
  aiModelSave(entry: AiModelEntry, key: string): Promise<AiModelsList>;
  aiModelDelete(id: string): Promise<AiModelsList>;
  aiModelUse(id: string): Promise<{ model: string }>;
  aiModelCheck(entry: AiModelEntry, key: string): Promise<AiCheckResult>;
  aiModelCandidates(entry: AiModelEntry, key: string): Promise<string[]>;
  onAiModelsChanged(handler: () => void): Promise<() => void>;
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
  filesList(id: string, path: string | null, follow?: boolean): Promise<Listing>;
  filesDownloadHere(id: string, remotePath: string, name: string, policy: ExistingPolicy, mode: TransferMode): Promise<void>;
  localList(path: string | null, follow?: boolean): Promise<LocalListing>;
  localPickDir(): Promise<LocalListing | null>;
  localRoots(): Promise<string[]>;
  localUpload(id: string, names: string[], remoteDir: string, policy: ExistingPolicy, mode: TransferMode): Promise<number>;
  syncGet(): Promise<boolean>;
  syncSet(enabled: boolean, sessionId: string | null, remotePath: string | null): Promise<void>;
  onLocalChanged(handler: () => void): Promise<() => void>;
  filesSearch(id: string, root: string, query: string): Promise<FileEntry[]>;
  filesJobs(): Promise<FileJob[]>;
  filesCancel(jobId: string): Promise<void>;
  filesClearJobs(): Promise<void>;
  filesUploadPick(id: string, remoteDir: string, folder: boolean, policy: ExistingPolicy, mode: TransferMode): Promise<void>;
  filesUploadDropped(id: string, remoteDir: string, policy: ExistingPolicy, mode: TransferMode): Promise<number>;
  filesDownload(id: string, remotePath: string, name: string, directory: boolean, policy: ExistingPolicy, mode: TransferMode): Promise<void>;
  filesDelete(id: string, remotePath: string, name: string, confirmation: string): Promise<void>;
  filesBookmarks(id: string): Promise<string[]>;
  filesBookmarkAdd(id: string, path: string): Promise<string[]>;
  onFileJob(handler: (job: FileJob) => void): Promise<() => void>;
  onFilesChanged(handler: (sessionId: string) => void): Promise<() => void>;
  onFileDrop(handler: (count: number) => void): Promise<() => void>;
  onFilesState(handler: (event: { id: string; available: boolean; message: string }) => void): Promise<() => void>;
  externalStatus(): Promise<ExternalStatus>;
  externalShare(mode: ShareMode, targets: string[]): Promise<ExternalStatus>;
  externalStop(): Promise<ExternalStatus>;
  mcpStart(): Promise<ExternalStatus>;
  mcpStop(): Promise<ExternalStatus>;
  permissionsStatus(id: string): Promise<PermissionStatus>;
  permissionsEnable(id: string, legacyStopped: boolean): Promise<PermissionStatus>;
  permissionsRecover(id: string): Promise<string>;
  onSessionWarning(handler: (event: { id: string; message: string }) => void): Promise<() => void>;
  editorOpenRemote(sessionId: string, path: string): Promise<OpenedDoc>;
  editorOpenLocal(): Promise<OpenedDoc | null>;
  editorSave(id: string, text: string | null, patches: [number, number][] | null): Promise<SavedDoc>;
  editorSaveAs(id: string, text: string | null, patches: [number, number][] | null): Promise<SavedDoc | null>;
  editorSetDirty(id: string, dirty: boolean): Promise<void>;
  editorClose(id: string): Promise<void>;
  appQuit(): Promise<void>;
  onCloseRequested(handler: () => void): Promise<() => void>;
  snippetsList(): Promise<Snippet[]>;
  snippetsSave(list: Snippet[]): Promise<Snippet[]>;
  snippetVariables(body: string): Promise<string[]>;
  snippetSend(id: string, index: number, values: Record<string, string>): Promise<void>;
  importLegacyInspect(legacyStopped: boolean): Promise<ImportPreview>;
  importBundlePick(): Promise<ImportPreview | null>;
  importBundleUnlock(token: string, passphrase: string): Promise<ImportPreview>;
  importCancel(): Promise<void>;
  importApply(options: ImportOptions): Promise<[ImportResult, Profile[]]>;
  secretsStatus(index: number): Promise<SecretsStatus>;
  secretsForget(index: number): Promise<void>;
  keygenDefaults(): Promise<KeygenDefaults>;
  keygenPathExists(path: string): Promise<boolean>;
  keygenPickPath(): Promise<string | null>;
  keygenGenerate(args: KeygenArgs): Promise<KeygenResult>;
  forwardList(id: string): Promise<ForwardInfo[]>;
  forwardStart(id: string, kind: string, bind: string, port: number, host: string, targetPort: number): Promise<ForwardInfo[]>;
  forwardStop(id: string, forwardId: string): Promise<ForwardInfo[]>;
  settingsGet(): Promise<AppSettings>;
  settingsSet(settings: AppSettings): Promise<void>;
  settingsPickLogDir(): Promise<string | null>;
  updateStatus(): Promise<UpdateInfo | null>;
  updateOpenPage(): Promise<void>;
  onUpdateAvailable(handler: (info: UpdateInfo) => void): Promise<() => void>;
  onSessionLogError(handler: (event: { id: string; message: string }) => void): Promise<() => void>;
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
    aiModelsList: () => invoke('ai_models_list'),
    aiModelSave: (entry, key) => invoke('ai_model_save', { entry, key }),
    aiModelDelete: (id) => invoke('ai_model_delete', { id }),
    aiModelUse: (id) => invoke('ai_model_use', { id }),
    aiModelCheck: (entry, key) => invoke('ai_model_check', { entry, key }),
    aiModelCandidates: (entry, key) => invoke('ai_model_candidates', { entry, key }),
    onAiModelsChanged: (handler) => listen('ai-models-changed', () => handler()),
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
    filesList: (id, path, follow) => invoke('files_list', { id, path, follow: follow ?? false }),
    filesDownloadHere: (id, remotePath, name, policy, mode) =>
      invoke('files_download_here', { id, remotePath, name, policy, mode }),
    localList: (path, follow) => invoke('local_list', { path, follow: follow ?? false }),
    localPickDir: () => invoke('local_pick_dir'),
    localRoots: () => invoke('local_roots'),
    localUpload: (id, names, remoteDir, policy, mode) => invoke('local_upload', { id, names, remoteDir, policy, mode }),
    syncGet: () => invoke('sync_get'),
    syncSet: (enabled, sessionId, remotePath) => invoke('sync_set', { enabled, sessionId, remotePath }),
    onLocalChanged: (handler) => listen('local-changed', () => handler()),
    filesSearch: (id, root, query) => invoke('files_search', { id, root, query }),
    filesJobs: () => invoke('files_jobs'),
    filesCancel: (jobId) => invoke('files_cancel', { jobId }),
    filesClearJobs: () => invoke('files_clear_jobs'),
    filesUploadPick: (id, remoteDir, folder, policy, mode) =>
      invoke('files_upload_pick', { id, remoteDir, folder, policy, mode }),
    filesUploadDropped: (id, remoteDir, policy, mode) =>
      invoke('files_upload_dropped', { id, remoteDir, policy, mode }),
    filesDownload: (id, remotePath, name, directory, policy, mode) =>
      invoke('files_download', { id, remotePath, name, directory, policy, mode }),
    filesDelete: (id, remotePath, name, confirmation) => invoke('files_delete', { id, remotePath, name, confirmation }),
    filesBookmarks: (id) => invoke('files_bookmarks', { id }),
    filesBookmarkAdd: (id, path) => invoke('files_bookmark_add', { id, path }),
    onFileJob: (handler) => listen<FileJob>('file-job', (e) => handler(e.payload)),
    onFilesChanged: (handler) => listen<string>('files-changed', (e) => handler(e.payload)),
    onFileDrop: (handler) => listen<number>('file-drop', (e) => handler(e.payload)),
    onFilesState: (handler) =>
      listen<{ id: string; available: boolean; message: string }>('files-state', (e) => handler(e.payload)),
    externalStatus: () => invoke('external_status'),
    externalShare: (mode, targets) => invoke('external_share', { mode, targets }),
    externalStop: () => invoke('external_stop'),
    mcpStart: () => invoke('mcp_start'),
    mcpStop: () => invoke('mcp_stop'),
    permissionsStatus: (id) => invoke('permissions_status', { id }),
    permissionsEnable: (id, legacyStopped) => invoke('permissions_enable', { id, legacyStopped }),
    permissionsRecover: (id) => invoke('permissions_recover', { id }),
    onSessionWarning: (handler) =>
      listen<{ id: string; message: string }>('session-warning', (e) => handler(e.payload)),
    editorOpenRemote: (sessionId, path) => invoke('editor_open_remote', { sessionId, path }),
    editorOpenLocal: () => invoke('editor_open_local'),
    editorSave: (id, text, patches) => invoke('editor_save', { id, text, patches }),
    editorSaveAs: (id, text, patches) => invoke('editor_save_as', { id, text, patches }),
    editorSetDirty: (id, dirty) => invoke('editor_set_dirty', { id, dirty }),
    editorClose: (id) => invoke('editor_close', { id }),
    appQuit: () => invoke('app_quit'),
    onCloseRequested: (handler) => listen('close-requested', () => handler()),
    snippetsList: () => invoke('snippets_list'),
    snippetsSave: (list) => invoke('snippets_save', { list }),
    snippetVariables: (body) => invoke('snippet_variables', { body }),
    snippetSend: (id, index, values) => invoke('snippet_send', { id, index, values }),
    importLegacyInspect: (legacyStopped) => invoke('import_legacy_inspect', { legacyStopped }),
    importBundlePick: () => invoke('import_bundle_pick'),
    importBundleUnlock: (token, passphrase) => invoke('import_bundle_unlock', { token, passphrase }),
    importCancel: () => invoke('import_cancel'),
    importApply: (options) => invoke('import_apply', { options }),
    secretsStatus: (index) => invoke('secrets_status', { index }),
    secretsForget: (index) => invoke('secrets_forget', { index }),
    keygenDefaults: () => invoke('keygen_defaults'),
    keygenPathExists: (path) => invoke('keygen_path_exists', { path }),
    keygenPickPath: () => invoke('keygen_pick_path'),
    keygenGenerate: (args) => invoke('keygen_generate', { args }),
    forwardList: (id) => invoke('forward_list', { id }),
    forwardStart: (id, kind, bind, port, host, targetPort) =>
      invoke('forward_start', { id, kind, bind, port, host, targetPort }),
    forwardStop: (id, forwardId) => invoke('forward_stop', { id, forwardId }),
    settingsGet: () => invoke('settings_get'),
    settingsSet: (settings) => invoke('settings_set', { settings }),
    settingsPickLogDir: () => invoke('settings_pick_log_dir'),
    updateStatus: () => invoke('update_status'),
    updateOpenPage: () => invoke('update_open_page'),
    onUpdateAvailable: (handler) => listen<UpdateInfo>('update-available', (e) => handler(e.payload)),
    onSessionLogError: (handler) =>
      listen<{ id: string; message: string }>('session-log-error', (e) => handler(e.payload)),
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
