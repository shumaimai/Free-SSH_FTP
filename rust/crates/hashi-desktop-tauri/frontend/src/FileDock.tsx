import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Badge,
  Body1,
  Button,
  Caption1,
  Checkbox,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  Input,
  makeStyles,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  mergeClasses,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  Popover,
  PopoverSurface,
  PopoverTrigger,
  ProgressBar,
  Radio,
  RadioGroup,
  SearchBox,
  Spinner,
  tokens,
} from '@fluentui/react-components';
import {
  ArrowDownload20Regular,
  ArrowSync20Regular,
  ArrowUp20Regular,
  ArrowUpload20Regular,
  Delete20Regular,
  Document20Regular,
  Edit20Regular,
  Dismiss16Regular,
  Folder20Filled,
  FolderArrowUp20Regular,
  Options20Regular,
  PanelLeft20Regular,
  ShieldKeyhole20Regular,
  Star20Regular,
} from '@fluentui/react-icons';
import type { Api } from './api';
import { PermissionDialog } from './PermissionDialog';
import { LocalPane } from './LocalPane';
import { Grip } from './Grip';
import type { ExistingPolicy, FileEntry, FileJob, Follow, LocalListing, TransferMode } from './types';

const useStyles = makeStyles({
  dual: { display: 'grid', columnGap: '3px', height: '100%', minHeight: 0 },
  ghost: {
    position: 'fixed',
    zIndex: 1000,
    pointerEvents: 'none',
    padding: '4px 10px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorBrandBackground,
    color: tokens.colorNeutralForegroundOnBrand,
    boxShadow: tokens.shadow8,
  },
  root: { display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, gap: '8px' },
  row: { display: 'flex', alignItems: 'center', gap: '6px' },
  wrap: { display: 'flex', flexWrap: 'wrap', gap: '6px', alignItems: 'center' },
  grow: { flex: 1, minWidth: 0 },
  list: {
    flex: 1,
    minHeight: 0,
    overflowY: 'auto',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  item: {
    display: 'flex',
    alignItems: 'center',
    gap: '8px',
    padding: '5px 10px',
    cursor: 'default',
    userSelect: 'none',
    contentVisibility: 'auto',
    containIntrinsicSize: '32px',
    ':hover': { backgroundColor: tokens.colorNeutralBackground1Hover },
  },
  itemOn: { backgroundColor: tokens.colorBrandBackground2, ':hover': { backgroundColor: tokens.colorBrandBackground2Hover } },
  name: { flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  dir: { color: tokens.colorBrandForeground1 },
  size: { color: tokens.colorNeutralForeground3, flexShrink: 0, fontSize: '12px' },
  jobs: {
    maxHeight: '170px',
    overflowY: 'auto',
    display: 'flex',
    flexDirection: 'column',
    gap: '6px',
    padding: '8px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  job: { display: 'flex', flexDirection: 'column', gap: '2px' },
  note: { color: tokens.colorNeutralForeground3 },
  error: { color: tokens.colorPaletteRedForeground1, overflowWrap: 'anywhere' },
});

export function humanSize(bytes: number): string {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[unit]}`;
}

/** 親フォルダのパス。ルートより上へは行かない。 */
export function parentOf(path: string): string {
  const trimmed = path.replace(/\/+$/, '');
  const index = trimmed.lastIndexOf('/');
  if (index <= 0) return '/';
  return trimmed.slice(0, index);
}

const PHASE: Record<FileJob['phase'], { label: string; color: 'brand' | 'success' | 'danger' | 'informative' | 'warning' }> = {
  queued: { label: '待機中', color: 'informative' },
  running: { label: '実行中', color: 'brand' },
  done: { label: '完了', color: 'success' },
  failed: { label: '失敗', color: 'danger' },
  cancelled: { label: '取消', color: 'warning' },
};

interface Props {
  /** ファイルを内蔵エディタで開く依頼（開く処理は呼び出し側）。 */
  onEdit: (sessionId: string, path: string) => void;
  /** 2ペイン（左にこのPCのフォルダ）の表示。ドックの幅は呼び出し側が変える。 */
  twoPane: boolean;
  /** 2ペインの左(ローカル)の割合。 */
  paneRatio: number;
  onPaneRatio(value: number): void;
  remoteHidden: boolean;
  onRemoteHidden(value: boolean): void;
  localHidden: boolean;
  onLocalHidden(value: boolean): void;
  onTwoPane: (value: boolean) => void;
  api: Api;
  sessionId: string | null;
  available: boolean;
  unavailableMessage: string;
}

export function FileDock({ api, sessionId, available, unavailableMessage, onEdit, twoPane, onTwoPane, paneRatio, onPaneRatio, remoteHidden: hidden, onRemoteHidden: setHidden, localHidden, onLocalHidden: setLocalHidden }: Props) {
  const [permOpen, setPermOpen] = useState(false);
  const styles = useStyles();
  const [path, setPath] = useState('');
  const [pathInput, setPathInput] = useState('');
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [searching, setSearching] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [policy, setPolicy] = useState<ExistingPolicy>('refuse');
  const [mode, setMode] = useState<TransferMode>('atomic');
  const [jobs, setJobs] = useState<FileJob[]>([]);
  const [bookmarks, setBookmarks] = useState<string[]>([]);
  const [deleteTarget, setDeleteTarget] = useState<FileEntry | null>(null);
  const [confirmation, setConfirmation] = useState('');
  const [query, setQuery] = useState('');
  const pathRef = useRef('');
  pathRef.current = path;
  const sessionRef = useRef<string | null>(sessionId);
  sessionRef.current = sessionId;

  // 2ペイン（ローカル側）。開いているフォルダはRust側が持ち、画面は表示と選んだ名前だけを持つ。
  const [local, setLocal] = useState<LocalListing | null>(null);
  const [localInput, setLocalInput] = useState('');
  const [localSel, setLocalSel] = useState<Set<string>>(new Set());
  const [localLoading, setLocalLoading] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [roots, setRoots] = useState<string[]>([]);
  const [syncOn, setSyncOn] = useState(false);
  const [notice, setNotice] = useState('');
  const anchor = useRef<string | null>(null);
  const loadRef = useRef<(target: string | null, follow?: boolean) => Promise<void>>(async () => {});
  const syncRef = useRef(false);
  syncRef.current = syncOn;
  const loadLocalRef = useRef<(target: string | null, follow?: boolean) => Promise<void>>(async () => {});

  /** 追随の指示を受けたら、もう片方のペインを動かす（失敗は通知だけにする）。 */
  const followUp = useCallback((follow: Follow | null) => {
    if (!follow) return;
    if (follow.kind === 'failed') setNotice(follow.message);
    else if (follow.kind === 'remote') void loadRef.current(follow.path, true);
    else void loadLocalRef.current(follow.path, true);
  }, []);

  const loadLocal = useCallback(
    async (target: string | null, follow = false) => {
      setLocalLoading(true);
      setLocalError(null);
      try {
        const listing = await api.localList(target, follow);
        setLocal(listing);
        setLocalInput(listing.path);
        setLocalSel(new Set());
        anchor.current = null;
        followUp(listing.follow);
      } catch (e) {
        const message = String(e);
        if (message.includes('置き換えられました')) return;
        if (follow) setNotice(message);
        else setLocalError(message);
      } finally {
        setLocalLoading(false);
      }
    },
    [api, followUp],
  );
  loadLocalRef.current = loadLocal;

  const load = useCallback(
    async (target: string | null, follow = false) => {
      if (!sessionId) return;
      setLoading(true);
      setError(null);
      try {
        const listing = await api.filesList(sessionId, target, follow);
        // 別の接続へ切り替わった後に届いた結果は捨てる。
        if (sessionRef.current !== sessionId) return;
        setPath(listing.path);
        setPathInput(listing.path);
        setEntries(listing.entries);
        setSearching(null);
        setSelected(null);
        followUp(listing.follow);
      } catch (e) {
        // 古い要求が新しい要求に置き換えられただけなら、エラーにしない。
        const message = String(e);
        if (message.includes('置き換えられました')) return;
        // 同期ブラウズの追随の失敗は、エラー欄ではなく通知にする。
        if (follow) setNotice(message);
        else setError(message);
      } finally {
        setLoading(false);
      }
    },
    [api, sessionId, followUp],
  );
  loadRef.current = load;

  // 接続が使えるようになったら、開始フォルダを開く。
  useEffect(() => {
    setPath('');
    setEntries([]);
    setError(null);
    setSearching(null);
    if (sessionId && available) {
      // 表示する接続が変わったら、同期ブラウズの基準を取り直す（最初の一覧で基準が決まる）。
      void api.syncSet(syncRef.current, sessionId, '').catch(() => {});
      void load(null);
      void api.filesBookmarks(sessionId).then(setBookmarks).catch(() => setBookmarks([]));
    }
  }, [sessionId, available, load, api]);

  useEffect(() => {
    void api.filesJobs().then(setJobs);
    const subs = [
      api.onFileJob((job) =>
        setJobs((old) => (old.some((j) => j.id === job.id) ? old.map((j) => (j.id === job.id ? job : j)) : [...old, job])),
      ),
      api.onFilesChanged((id) => {
        // 転送や削除が終わったら、開いているフォルダを読み直す。
        if (id === sessionRef.current && pathRef.current) void load(pathRef.current);
      }),
      api.onFileDrop(({ count, x, y }) => {
        const id = sessionRef.current;
        if (!id || count === 0 || !pathRef.current) return;
        // ファイル一覧(リモート)の上に落としたときだけ送る。端末などへの落とし損ねで送らない。
        const box = remoteBox.current?.getBoundingClientRect();
        const inside = box && box.width > 0 && x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
        if (!inside) {
          setNotice('送信するファイルは、リモートのファイル一覧の上にドロップしてください');
          return;
        }
        void api.filesUploadDropped(id, pathRef.current, policy, mode).catch((e) => setError(String(e)));
      }),
    ];
    return () => subs.forEach((p) => void p.then((off) => off()));
  }, [api, load, policy, mode]);

  // 2ペインを開いたら、ローカルのフォルダを読み込む。同期ブラウズの設定も読む。
  useEffect(() => {
    if (!twoPane) return;
    void api.syncGet().then(setSyncOn).catch(() => setSyncOn(false));
    if (!local) void loadLocal(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [twoPane, api]);

  useEffect(() => {
    const sub = api.onLocalChanged(() => {
      if (twoPaneRef.current) void loadLocalRef.current(null);
    });
    return () => void sub.then((off) => off());
  }, [api]);

  const dualBox = useRef<HTMLDivElement>(null);
  const remoteBox = useRef<HTMLDivElement>(null);
  const ratioStart = useRef(0.5);
  const twoPaneRef = useRef(twoPane);
  twoPaneRef.current = twoPane;

  const localShown = useMemo(
    () => (local?.entries ?? []).filter((e) => localHidden || !e.name.startsWith('.')),
    [local, localHidden],
  );

  const setSync = (value: boolean) => {
    setSyncOn(value);
    syncRef.current = value;
    setNotice('');
    void api.syncSet(value, sessionRef.current, pathRef.current || '').catch((e) => setError(String(e)));
  };

  const selectLocal = (entry: FileEntry, additive: boolean, range: boolean) => {
    setLocalSel((old) => {
      if (range && anchor.current) {
        const names = localShown.map((e) => e.name);
        const [a, b] = [names.indexOf(anchor.current), names.indexOf(entry.name)];
        if (a >= 0 && b >= 0) return new Set(names.slice(Math.min(a, b), Math.max(a, b) + 1));
      }
      anchor.current = entry.name;
      if (additive) {
        const next = new Set(old);
        if (next.has(entry.name)) next.delete(entry.name);
        else next.add(entry.name);
        return next;
      }
      return new Set([entry.name]);
    });
  };

  const sendSelected = () => {
    const id = sessionRef.current;
    const names = [...localSelRef.current];
    if (!id || !pathRef.current || names.length === 0) return;
    void api.localUpload(id, names, pathRef.current, policy, mode).catch((e) => setError(String(e)));
  };
  const receiveSelected = () => {
    const id = sessionRef.current;
    const entry = entries.find((e) => e.path === selectedRef.current);
    if (!id || !entry) return;
    void api.filesDownloadHere(id, entry.path, entry.name, policy, mode).catch((e) => setError(String(e)));
  };
  const localSelRef = useRef(localSel);
  localSelRef.current = localSel;
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const transferRef = useRef<(from: 'local' | 'remote') => void>(() => {});
  transferRef.current = (from) => (from === 'local' ? sendSelected() : receiveSelected());

  // ペイン間のドラッグ。運ぶのは「どちらのペインか」の印だけで、何を送るかは受け側が選択状態から決める。
  // （Tauriのファイルドロップ受信を有効にしたままでもWindowsで動くよう、HTML5のドラッグ&ドロップは使わない。）
  const [drag, setDrag] = useState<{ x: number; y: number; label: string } | null>(null);
  const dragRef = useRef<{ pane: 'local' | 'remote'; sx: number; sy: number; active: boolean; label: string } | null>(null);
  useEffect(() => {
    const move = (e: PointerEvent) => {
      const d = dragRef.current;
      if (!d) return;
      if (!d.active && Math.hypot(e.clientX - d.sx, e.clientY - d.sy) > 8) {
        d.active = true;
        // 掴んでいる間は文字が選択されないようにする。
        document.body.style.userSelect = 'none';
      }
      if (d.active) setDrag({ x: e.clientX, y: e.clientY, label: d.label });
    };
    const up = (e: PointerEvent) => {
      const d = dragRef.current;
      dragRef.current = null;
      document.body.style.userSelect = '';
      setDrag(null);
      if (!d?.active) return;
      const under = document.elementFromPoint(e.clientX, e.clientY);
      const pane = (under?.closest('[data-pane]') as HTMLElement | null)?.dataset.pane;
      if (pane && pane !== d.pane) transferRef.current(d.pane);
    };
    const cancel = () => {
      dragRef.current = null;
      document.body.style.userSelect = '';
      setDrag(null);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', cancel);
    return () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', cancel);
    };
  }, []);
  const startDrag = (pane: 'local' | 'remote', label: string, e: React.PointerEvent) => {
    if (e.button !== 0 || !twoPaneRef.current) return;
    dragRef.current = { pane, sx: e.clientX, sy: e.clientY, active: false, label };
  };

  const shown = useMemo(() => {
    const list = entries.filter((e) => hidden || searching !== null || !e.name.startsWith('.'));
    return [...list].sort((a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name, 'ja'));
  }, [entries, hidden, searching]);

  const current = entries.find((e) => e.path === selected) ?? null;

  const enter = (entry: FileEntry) => {
    if (entry.directory) void load(entry.path);
    else if (sessionId) onEdit(sessionId, entry.path);
  };
  const run = (task: Promise<unknown>) => task.catch((e) => setError(String(e)));

  if (!sessionId) {
    return <Body1 className={styles.note}>SSHで接続したサーバーを開くと、ここにファイルが並びます。</Body1>;
  }
  if (!available) {
    return (
      <MessageBar intent="warning">
        <MessageBarBody>
          {unavailableMessage ? `ファイルを開けません: ${unavailableMessage}` : '接続が済むと、ここにファイルが並びます。'}
        </MessageBarBody>
      </MessageBar>
    );
  }

  const activeJobs = jobs.filter((j) => j.sessionId === sessionId);
  const remotePane = (
    <div className={styles.root} data-pane="remote" ref={remoteBox}>
      <div className={styles.row}>
        <Button
          size="small"
          icon={<ArrowUp20Regular />}
          aria-label="親フォルダへ"
          disabled={!path || path === '/'}
          onClick={() => void load(parentOf(path))}
        />
        <Button size="small" icon={<ArrowSync20Regular />} aria-label="再読み込み" onClick={() => void load(path)} />
        <Input
          className={styles.grow}
          size="small"
          aria-label="フォルダのパス"
          value={pathInput}
          onChange={(_, d) => setPathInput(d.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing) void load(pathInput);
          }}
        />
        <Menu>
          <MenuTrigger disableButtonEnhancement>
            <Button size="small" icon={<Star20Regular />} aria-label="ブックマーク" />
          </MenuTrigger>
          <MenuPopover>
            <MenuList>
              <MenuItem
                onClick={() => void run(api.filesBookmarkAdd(sessionId, path).then(setBookmarks))}
                disabled={!path}
              >
                現在のフォルダを登録
              </MenuItem>
              {bookmarks.map((b) => (
                <MenuItem key={b} onClick={() => void load(b)}>
                  {b}
                </MenuItem>
              ))}
            </MenuList>
          </MenuPopover>
        </Menu>
      </div>
      <div className={styles.wrap}>
        <Button
          appearance="primary"
          size="small"
          icon={<ArrowDownload20Regular />}
          disabled={!current}
          onClick={() =>
            current &&
            void run(
              twoPane
                ? api.filesDownloadHere(sessionId, current.path, current.name, policy, mode)
                : api.filesDownload(sessionId, current.path, current.name, current.directory, policy, mode),
            )
          }
        >
          {twoPane ? '受信（左のフォルダへ）' : '受信'}
        </Button>
        <Button
          size="small"
          icon={<Edit20Regular />}
          disabled={!current || current.directory}
          onClick={() => current && onEdit(sessionId, current.path)}
        >
          編集
        </Button>
        <Button
          size="small"
          icon={<ArrowUpload20Regular />}
          disabled={!path}
          onClick={() => void run(api.filesUploadPick(sessionId, path, false, policy, mode))}
        >
          送信
        </Button>
        <Button
          size="small"
          icon={<FolderArrowUp20Regular />}
          disabled={!path}
          onClick={() => void run(api.filesUploadPick(sessionId, path, true, policy, mode))}
        >
          フォルダ送信
        </Button>
        <Button
          size="small"
          appearance="subtle"
          icon={<Delete20Regular />}
          style={{ color: tokens.colorPaletteRedForeground1 }}
          disabled={!current}
          onClick={() => {
            setDeleteTarget(current);
            setConfirmation('');
          }}
        >
          削除
        </Button>
        <div className={styles.grow} />
        <Button
          size="small"
          appearance={twoPane ? 'primary' : 'subtle'}
          icon={<PanelLeft20Regular />}
          aria-pressed={twoPane}
          onClick={() => onTwoPane(!twoPane)}
        >
          2ペイン
        </Button>
        <Button size="small" appearance="subtle" icon={<ShieldKeyhole20Regular />} onClick={() => setPermOpen(true)}>
          権限補助
        </Button>
        <Popover positioning="below-end">
          <PopoverTrigger disableButtonEnhancement>
            <Button size="small" appearance="subtle" icon={<Options20Regular />}>
              転送の設定
            </Button>
          </PopoverTrigger>
          <PopoverSurface style={{ width: 320 }}>
            <Field label="すでに同名のファイルがあるとき">
              <RadioGroup value={policy} onChange={(_, d) => setPolicy(d.value as ExistingPolicy)}>
                <Radio value="refuse" label="中止する" />
                <Radio value="overwrite" label="上書きする" />
                <Radio value="resume" label="続きから送る（途中のファイルへ追記）" />
                <Radio value="skip" label="飛ばす" />
              </RadioGroup>
            </Field>
            <Field label="書き込み方式" style={{ marginTop: 8 }}>
              <RadioGroup value={mode} onChange={(_, d) => setMode(d.value as TransferMode)}>
                <Radio value="atomic" label="原本を守る（既定）" />
                <Radio value="in_place" label="直接書込み（Python互換）" />
              </RadioGroup>
            </Field>
            <Caption1 className={mode === 'in_place' ? styles.error : styles.note}>
              {mode === 'in_place'
                ? '直接書込みは取消・失敗でも転送先が変更され、部分データが残る場合があります。'
                : '原本を守る方式には、親フォルダへの書込み権限とサーバーの対応が必要です。'}
            </Caption1>
          </PopoverSurface>
        </Popover>
      </div>
      <div className={styles.row}>
        <SearchBox
          className={styles.grow}
          size="small"
          placeholder="このフォルダ以下を検索"
          value={query}
          onChange={(_, d) => setQuery(d.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && !e.nativeEvent.isComposing && query.trim() && path) {
              setLoading(true);
              api
                .filesSearch(sessionId, path, query.trim())
                .then((found) => {
                  setEntries(found);
                  setSearching(query.trim());
                  setSelected(null);
                })
                .catch((err) => setError(String(err)))
                .finally(() => setLoading(false));
            }
          }}
        />
        <Checkbox checked={hidden} onChange={(_, d) => setHidden(d.checked === true)} label="隠しファイル" />
      </div>
      {searching !== null && (
        <MessageBar intent="info">
          <MessageBarBody>
            「{searching}」の検索結果 {entries.length}件
            <Button size="small" appearance="transparent" icon={<Dismiss16Regular />} onClick={() => void load(path)}>
              閉じる
            </Button>
          </MessageBarBody>
        </MessageBar>
      )}
      {error && (
        <MessageBar intent="error">
          <MessageBarBody>{error}</MessageBarBody>
        </MessageBar>
      )}
      <div className={styles.list} role="listbox" aria-label="ファイル一覧">
        {loading && shown.length === 0 && (
          <div style={{ padding: 12 }}>
            <Spinner size="tiny" label="読み込み中" />
          </div>
        )}
        {!loading && shown.length === 0 && <Caption1 className={styles.note} style={{ padding: 12, display: 'block' }}>空のフォルダです</Caption1>}
        {shown.map((entry) => (
          <div
            key={entry.path}
            role="option"
            aria-selected={selected === entry.path}
            tabIndex={0}
            className={mergeClasses(styles.item, selected === entry.path && styles.itemOn)}
            onPointerDown={(e) => {
              // 選択していない行を掴んだら、その行だけを選んで運ぶ。
              if (selectedRef.current !== entry.path) setSelected(entry.path);
              selectedRef.current = entry.path;
              startDrag('remote', entry.name, e);
            }}
            onClick={() => setSelected(entry.path)}
            onDoubleClick={() => enter(entry)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') enter(entry);
              if (e.key === ' ') {
                e.preventDefault();
                setSelected(entry.path);
              }
            }}
          >
            {entry.directory ? <Folder20Filled className={styles.dir} /> : <Document20Regular />}
            <span className={mergeClasses(styles.name, entry.directory && styles.dir)} title={entry.name}>
              {entry.name}
              {entry.symlink && ' （リンク）'}
            </span>
            {!entry.directory && <span className={styles.size}>{humanSize(entry.size)}</span>}
          </div>
        ))}
      </div>
      {activeJobs.length > 0 && (
        <div className={styles.jobs} aria-label="転送">
          <div className={styles.row}>
            <Caption1 className={styles.grow}>転送</Caption1>
            <Button size="small" appearance="subtle" onClick={() => void api.filesClearJobs().then(() => api.filesJobs().then(setJobs))}>
              完了分を消す
            </Button>
          </div>
          {activeJobs.map((job) => (
            <div key={job.id} className={styles.job}>
              <div className={styles.row}>
                <Caption1 className={styles.name} title={job.label}>
                  {job.label}
                </Caption1>
                <Badge appearance="tint" color={PHASE[job.phase].color}>
                  {PHASE[job.phase].label}
                </Badge>
                {(job.phase === 'queued' || job.phase === 'running') && (
                  <Button size="small" appearance="subtle" onClick={() => void api.filesCancel(job.id)}>
                    取消
                  </Button>
                )}
              </div>
              {job.phase === 'running' && job.total > 0 && <ProgressBar value={job.done / job.total} thickness="medium" />}
              {job.message && <Caption1 className={job.phase === 'failed' ? styles.error : styles.note}>{job.message}</Caption1>}
            </div>
          ))}
        </div>
      )}
      <Dialog open={deleteTarget !== null} onOpenChange={(_, d) => !d.open && setDeleteTarget(null)}>
        <DialogSurface aria-describedby={undefined}>
          <DialogBody>
            <DialogTitle>削除の確認</DialogTitle>
            <DialogContent>
              <Body1>
                {deleteTarget?.name}
                {deleteTarget?.directory ? ' とその中身をすべて削除します。' : ' を削除します。'}元に戻せません。
              </Body1>
              <Field label="実行するには delete と入力してください" style={{ marginTop: 10 }}>
                <Input value={confirmation} onChange={(_, d) => setConfirmation(d.value)} />
              </Field>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setDeleteTarget(null)}>やめる</Button>
              <Button
                appearance="primary"
                disabled={confirmation !== 'delete'}
                onClick={() => {
                  const target = deleteTarget;
                  setDeleteTarget(null);
                  if (target) void run(api.filesDelete(sessionId, target.path, target.name, confirmation));
                }}
              >
                削除する
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <PermissionDialog api={api} open={permOpen} sessionId={sessionId} onClose={() => setPermOpen(false)} />
    </div>
  );
  return (
    <div
      ref={dualBox}
      className={twoPane ? styles.dual : undefined}
      style={
        twoPane
          ? { gridTemplateColumns: `minmax(0,${paneRatio}fr) 6px minmax(0,${1 - paneRatio}fr)` }
          : { height: '100%' }
      }
    >
      {twoPane && (
        <LocalPane
          path={local?.path ?? ''}
          pathInput={localInput}
          parent={local?.parent ?? null}
          entries={[...localShown].sort(
            (a, b) => Number(b.directory) - Number(a.directory) || a.name.localeCompare(b.name, 'ja'),
          )}
          selected={localSel}
          hidden={localHidden}
          loading={localLoading}
          error={localError}
          sync={syncOn}
          roots={roots}
          canSend={Boolean(path)}
          onPathInput={setLocalInput}
          onGo={(target) => void loadLocal(target)}
          onRefresh={() => void loadLocal(local?.path ?? null)}
          onPick={() =>
            void api
              .localPickDir()
              .then((listing) => {
                if (!listing) return;
                setLocal(listing);
                setLocalInput(listing.path);
                setLocalSel(new Set());
                followUp(listing.follow);
              })
              .catch((e) => setLocalError(String(e)))
          }
          onRoots={() => void api.localRoots().then(setRoots).catch(() => setRoots([]))}
          onEnter={(entry) => entry.directory && void loadLocal(entry.path)}
          onSelect={selectLocal}
          onRowPointerDown={(entry, e) => {
            if (!localSelRef.current.has(entry.name)) {
              setLocalSel(new Set([entry.name]));
              localSelRef.current = new Set([entry.name]);
            }
            const n = Math.max(localSelRef.current.size, 1);
            startDrag('local', n > 1 ? `${n}件` : entry.name, e);
          }}
          onHidden={setLocalHidden}
          onSync={setSync}
          onSend={sendSelected}
        />
      )}
      {twoPane && (
        <Grip
          label="ローカルとリモートの幅"
          onBegin={() => {
            ratioStart.current = paneRatio;
          }}
          onMove={(dx) => {
            const width = dualBox.current?.clientWidth ?? 0;
            if (width <= 0) return;
            onPaneRatio(Math.min(0.85, Math.max(0.15, ratioStart.current + dx / width)));
          }}
          onReset={() => onPaneRatio(0.5)}
        />
      )}
      {remotePane}
      {notice && (
        <div style={{ position: 'fixed', bottom: 36, right: 16, zIndex: 900, maxWidth: 420 }}>
          <MessageBar intent="warning">
            <MessageBarBody>{notice}</MessageBarBody>
            <MessageBarActions>
              <Button size="small" appearance="transparent" onClick={() => setNotice('')}>
                閉じる
              </Button>
            </MessageBarActions>
          </MessageBar>
        </div>
      )}
      {drag && (
        <div className={styles.ghost} style={{ left: drag.x + 12, top: drag.y + 12 }}>
          {drag.label}
        </div>
      )}
    </div>
  );
}
