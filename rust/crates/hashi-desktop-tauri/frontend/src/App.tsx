import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  FluentProvider,
  Menu,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  makeStyles,
  mergeClasses,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  Tab,
  TabList,
  Text,
  ToggleButton,
  Tooltip,
  tokens,
} from '@fluentui/react-components';
import { Icon } from './icons';
import type { Api } from './api';
import type { OpenedDoc, UpdateInfo, ApprovalRequest, Challenge, ChallengeAnswer, Profile, SessionInfo, SessionState } from './types';
import { emptyProfile } from './types';
import { Home } from './Home';
import { ProfileDialog } from './ProfileDialog';
import { ChallengeDialog } from './ChallengeDialog';
import { TerminalView } from './TerminalView';
import { SettingsDialog } from './SettingsDialog';
import { AiModelsPage } from './AiModelsPage';
import { AiDock } from './AiDock';
import { FileDock } from './FileDock';
import { SnippetBar } from './SnippetBar';
import { EditorPane } from './EditorView';
import { KeygenDialog } from './KeygenDialog';
import { ImportDialog } from './ImportDialog';
import { ErrorBoundary } from './ErrorBoundary';
import { ForwardDialog } from './ForwardDialog';
import { ExternalDialog } from './ExternalDialog';
import { ApprovalDialog } from './ApprovalDialog';
import { Grip } from './Grip';
import { rustHas, t, useLanguage } from './i18n';
import { DEFAULT_APPEARANCE, UI_FONTS, fluentTheme, sanitize, type Appearance } from './appearance';

/** ファイル/AIパネルの既定幅と下限、端末側に残す最低幅(px)。 */
const DOCK_WIDTH = 420;
const DUAL_WIDTH = 980;
const MIN_DOCK = 280;
const MIN_MAIN = 360;

const useStyles = makeStyles({
  shell: {
    display: 'grid',
    gridTemplateRows: 'auto 1fr auto',
    height: '100vh',
    backgroundColor: tokens.colorNeutralBackground2,
    color: tokens.colorNeutralForeground1,
  },
  bar: {
    display: 'flex',
    alignItems: 'center',
    gap: '8px',
    padding: '6px 12px 0 12px',
    backgroundColor: tokens.colorNeutralBackground3,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
    minHeight: '44px',
    overflowX: 'auto',
  },
  brand: { fontWeight: tokens.fontWeightSemibold, color: tokens.colorBrandForeground1, marginRight: '8px' },
  tabClose: {
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    width: '18px',
    height: '18px',
    marginLeft: '6px',
    borderRadius: tokens.borderRadiusSmall,
    ':hover': { backgroundColor: tokens.colorNeutralBackground1Hover },
  },
  work: { display: 'flex', minHeight: 0 },
  main: { position: 'relative', minHeight: 0, minWidth: 0, flex: 1, backgroundColor: tokens.colorNeutralBackground2 },
  dock: {
    minWidth: `${MIN_DOCK}px`,
    flexShrink: 0,
    padding: '12px',
    boxSizing: 'border-box',
    backgroundColor: tokens.colorNeutralBackground3,
    borderLeft: `1px solid ${tokens.colorNeutralStroke2}`,
    minHeight: 0,
    overflow: 'hidden',
  },
  pane: { position: 'absolute', inset: 0 },
  paneCol: { display: 'flex', flexDirection: 'column' },
  termArea: { position: 'relative', flex: 1, minHeight: 0 },
  paneHidden: { visibility: 'hidden', pointerEvents: 'none' },
  status: {
    display: 'flex',
    alignItems: 'center',
    gap: '12px',
    padding: '4px 14px',
    backgroundColor: tokens.colorNeutralBackground3,
    borderTop: `1px solid ${tokens.colorNeutralStroke2}`,
    minHeight: '26px',
  },
  grow: { flex: 1 },
  banner: { position: 'absolute', top: '8px', left: '16px', right: '16px', zIndex: 5 },
});

interface TabInfo {
  key: string;
  kind: 'local' | 'ssh' | 'editor' | 'aimodels';
  index?: number;
  label: string;
  sessionId?: string;
  /** エディタのタブ: 開いている文書と未保存の印。 */
  doc?: OpenedDoc;
  dirty?: boolean;
}

function useDark(): boolean {
  const query = useMemo(() => window.matchMedia('(prefers-color-scheme: dark)'), []);
  const [dark, setDark] = useState(query.matches);
  useEffect(() => {
    const handler = (e: MediaQueryListEvent) => setDark(e.matches);
    query.addEventListener('change', handler);
    return () => query.removeEventListener('change', handler);
  }, [query]);
  return dark;
}

export function App({ api }: { api: Api }) {
  const styles = useStyles();
  useLanguage(); // 言語を切り替えたら全体を描き直す
  const systemDark = useDark();
  const [appearance, setAppearance] = useState<Appearance>(DEFAULT_APPEARANCE);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [dockOpen, setDockOpen] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [keygenOpen, setKeygenOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [savedSecrets, setSavedSecrets] = useState<string[]>([]);
  const [forwardOpen, setForwardOpen] = useState(false);
  const [externalOpen, setExternalOpen] = useState(false);
  const [twoPane, setTwoPane] = useState(false);
  const [dockTab, setDockTab] = useState<'files' | 'ai'>('files');
  const [remoteHidden, setRemoteHidden] = useState(false);
  const [localHidden, setLocalHidden] = useState(false);
  // パネルの幅(1ペイン用/2ペイン用)と、2ペインの左右の割合。つまみのドラッグで変える。
  const [dockWidth, setDockWidth] = useState(DOCK_WIDTH);
  const [dualWidth, setDualWidth] = useState(DUAL_WIDTH);
  const [paneRatio, setPaneRatio] = useState(0.5);
  const dragStart = useRef(0);
  // 保存済みの小さな表示設定を読み込み終えるまでは、書き戻さない。
  const [prefsLoaded, setPrefsLoaded] = useState(false);
  const [filesState, setFilesState] = useState<Record<string, { available: boolean; message: string }>>({});
  const autoOpened = useRef(false);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const wide = twoPane && dockTab === 'files';
  const dark = appearance.mode === 'dark' || (appearance.mode === 'system' && systemDark);
  const theme = useMemo(() => {
    const base = fluentTheme(dark, appearance.accent);
    const font = (UI_FONTS.find((f) => f.id === appearance.uiFont) ?? UI_FONTS[0]).stack;
    return { ...base, fontFamilyBase: font };
  }, [dark, appearance.accent, appearance.uiFont]);
  const changeAppearance = (value: Appearance) => {
    const next = sanitize(value);
    setAppearance(next);
    void api.appearanceSet(next).catch((e) => setNotice(String(e)));
  };
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [tabs, setTabs] = useState<TabInfo[]>([]);
  const tabsRef = useRef<TabInfo[]>([]);
  tabsRef.current = tabs;
  const [active, setActive] = useState<string>('home');
  const [states, setStates] = useState<Record<string, { state: SessionState; message: string }>>({});
  const [notice, setNotice] = useState('');
  const [dialog, setDialog] = useState<{ index: number | null; profile: Profile; error: string | null } | null>(null);
  const [challenges, setChallenges] = useState<Challenge[]>([]);
  const [challengeError, setChallengeError] = useState<string | null>(null);
  const closed = useRef(new Set<string>());

  useEffect(() => {
    void api.appearanceGet().then((a) => a && setAppearance(sanitize(a)));
    void api
      .uiPrefsGet()
      .then((p) => {
        if (p) {
          setRemoteHidden(p.remoteHidden);
          setLocalHidden(p.localHidden);
          setTwoPane(p.twoPane);
          setDockOpen(p.dockOpen);
          setDockTab(p.dockTab === 'ai' ? 'ai' : 'files');
          if (Number.isFinite(p.dockWidth)) setDockWidth(p.dockWidth);
          if (Number.isFinite(p.dualWidth)) setDualWidth(p.dualWidth);
          if (Number.isFinite(p.paneRatio)) setPaneRatio(p.paneRatio);
          // 前回の状態を尊重する。接続のたびに勝手に開き直さない。
          autoOpened.current = true;
        }
      })
      .catch(() => undefined)
      .finally(() => setPrefsLoaded(true));
    void api.profilesList().then((r) => {
      setProfiles(r.profiles);
      setStartupError(r.error);
    });
    void api.updateStatus().then((u) => u && setUpdate(u));
    const subscriptions: Array<Promise<() => void>> = [
      api.onUpdateAvailable(setUpdate),
      api.onSessionLogError((e) => setNotice(e.message)),
      api.onFilesState((e) => {
        setFilesState((old) => ({ ...old, [e.id]: { available: e.available, message: e.message } }));
        // 最初にファイルが使えるようになったとき、一度だけパネルを開く。
        if (e.available && !autoOpened.current) {
          autoOpened.current = true;
          setDockTab('files');
          setDockOpen(true);
        }
      }),
      api.onCloseRequested(() => setQuitAsk(true)),
      api.onSessionWarning((e) => setWarnings((old) => [...old, e.message].slice(-5))),
      api.onApproval((r) => setApprovals((old) => [...old, r])),
      api.onApprovalClosed((id) => setApprovals((old) => old.filter((r) => r.id !== id))),
      api.onChallenge((c) => setChallenges((old) => [...old, c])),
      api.onSessionState((e) => {
        setStates((old) => ({ ...old, [e.id]: { state: e.state, message: e.message } }));
        // 終わった接続の確認画面は片付ける。
        if (e.state === 'closed' || e.state === 'error') {
          setChallenges((old) => old.filter((c) => c.scope.stamp.terminal_id !== e.id));
        }
      }),
    ];
    return () => subscriptions.forEach((s) => void s.then((off) => off()));
  }, [api]);

  useEffect(() => {
    if (!prefsLoaded) return;
    // つまみのドラッグ中に毎回書き込まないよう、変化が落ち着いてから保存する。
    const timer = window.setTimeout(() => {
      void api
        .uiPrefsSet({
          remoteHidden,
          localHidden,
          twoPane,
          dockOpen,
          dockTab,
          dockWidth: Math.round(dockWidth),
          dualWidth: Math.round(dualWidth),
          paneRatio,
        })
        .catch(() => undefined);
    }, 400);
    return () => window.clearTimeout(timer);
  }, [api, prefsLoaded, remoteHidden, localHidden, twoPane, dockOpen, dockTab, dockWidth, dualWidth, paneRatio]);

  const openTab = useCallback((tab: Omit<TabInfo, 'key'>) => {
    const key = crypto.randomUUID();
    setTabs((old) => [...old, { ...tab, key }]);
    setActive(key);
  }, []);

  // AIモデルの管理ページは1つだけ開く。すでにあればそのタブへ移る。
  const openAiModels = useCallback(() => {
    const existing = tabsRef.current.find((t) => t.kind === 'aimodels');
    if (existing) setActive(existing.key);
    else openTab({ kind: 'aimodels', label: t('AIモデル') });
  }, [openTab]);

  const [discard, setDiscard] = useState<string | null>(null);
  const [quitAsk, setQuitAsk] = useState(false);
  const [warnings, setWarnings] = useState<string[]>([]);

  const closeTab = useCallback(
    (key: string, force = false) => {
      const tab = tabs.find((t) => t.key === key);
      if (tab?.kind === 'editor' && tab.dirty && !force) {
        setActive(key);
        setDiscard(key);
        return;
      }
      if (tab?.doc) void api.editorClose(tab.doc.id);
      closed.current.add(key);
      setTabs((old) => old.filter((t) => t.key !== key));
      // ブラウザと同じく、閉じたタブの1つ左へ移る(左が無ければホーム)。
      const index = tabs.findIndex((t) => t.key === key);
      const left = index > 0 ? tabs[index - 1].key : 'home';
      setActive((current) => (current === key ? left : current));
      if (tab?.sessionId) void api.close(tab.sessionId).catch((e) => setNotice(String(e)));
    },
    [api, tabs],
  );

  const openEditor = useCallback(
    (task: Promise<OpenedDoc | null>) => {
      task
        .then((doc) => doc && openTab({ kind: 'editor', label: doc.name, doc, dirty: false }))
        .catch((e) => setNotice(String(e)));
    },
    [openTab],
  );

  const onOpened = useCallback(
    (key: string, info: SessionInfo) => {
      if (closed.current.has(key)) {
        void api.close(info.id);
        return;
      }
      setTabs((old) => old.map((t) => (t.key === key ? { ...t, sessionId: info.id, label: info.label } : t)));
    },
    [api],
  );

  const saveProfile = async (profile: Profile, connect: boolean) => {
    if (!dialog) return;
    try {
      const before = profiles.length;
      const list = await api.profileSave(dialog.index, profile);
      setProfiles(list);
      setDialog(null);
      setNotice(t('保存しました'));
      if (connect) {
        const index = dialog.index ?? (list.length > before ? list.length - 1 : list.length - 1);
        openTab({ kind: 'ssh', index, label: profile.name || profile.host });
      }
    } catch (error) {
      setDialog({ ...dialog, error: String(error) });
    }
  };

  const answer = async (a: ChallengeAnswer) => {
    const current = challenges[0];
    if (!current) return;
    try {
      await api.challengeReply(current.challenge_id, a);
      setChallengeError(null);
      setChallenges((old) => old.filter((c) => c.challenge_id !== current.challenge_id));
    } catch (error) {
      const message = String(error);
      setChallengeError(message);
      // 終了した要求への応答は、再試行しても通らない。
      if (rustHas(error, '終了')) { // i18n-ignore: Rust側のエラー文(日本語)を見分ける
        setChallenges((old) => old.filter((c) => c.challenge_id !== current.challenge_id));
      }
    }
  };

  // 接続できている端末だけを、AIへ見せる候補にする。
  const shareTargets = useMemo(
    () =>
      tabs
        .filter((t) => t.sessionId && states[t.sessionId]?.state === 'connected')
        .map((t) => ({ sessionId: t.sessionId as string, label: t.label })),
    [tabs, states],
  );
  const activeTab = tabs.find((t) => t.key === active);
  // ファイル欄は、いま開いているSSH接続のもの。ローカル端末やホームでは出さない。
  const tabSession = activeTab?.kind === 'editor' ? (activeTab.doc?.sessionId ?? undefined) : activeTab?.sessionId;
  const filesSessionId =
    activeTab && activeTab.kind !== 'local' && tabSession && states[tabSession]?.state === 'connected'
      ? tabSession
      : null;
  const activeState = tabSession ? states[tabSession] : undefined;
  // 接続の状態は色だけでなく形(アイコン)と名前でも示す。
  const StateMark = ({ tab }: { tab: TabInfo }) => {
    const s = tab.sessionId ? states[tab.sessionId]?.state : undefined;
    const [Mark, color, label] =
      s === 'connected'
        ? [Icon.stateConnected, tokens.colorPaletteGreenForeground1, t('接続済み')]
        : s === 'error'
          ? [Icon.stateError, tokens.colorPaletteRedForeground1, t('エラー')]
          : s === 'closed'
            ? [Icon.stateClosed, tokens.colorNeutralForeground4, t('切断')]
            : [Icon.stateConnecting, tokens.colorPaletteYellowForeground1, t('接続しています')];
    return (
      <Mark
        role="img"
        aria-label={label}
        title={label}
        style={{ color, marginRight: 6, flexShrink: 0 }}
      />
    );
  };

  return (
    <FluentProvider theme={theme} style={{ height: '100vh' }}>
      <div className={styles.shell}>
        <div className={styles.bar}>
          <span className={styles.brand}>Hashi</span>
          <TabList
            selectedValue={active}
            onTabSelect={(_, d) => setActive(String(d.value))}
            size="medium"
          >
            <Tab value="home" icon={<Icon.home />}>
              {t('ホーム')}
            </Tab>
            {tabs.map((tab) => (
              <Tab
                key={tab.key}
                value={tab.key}
                icon={
                  tab.kind === 'ssh' ? (
                    <Icon.sshSession />
                  ) : tab.kind === 'editor' ? (
                    <Icon.editFile />
                  ) : tab.kind === 'aimodels' ? (
                    <Icon.aiModels />
                  ) : (
                    <Icon.localTerminal />
                  )
                }
              >
                <span style={{ display: 'inline-flex', alignItems: 'center' }}>
                  {tab.kind !== 'editor' && tab.kind !== 'aimodels' && <StateMark tab={tab} />}
                  {tab.label}
                  {tab.dirty && <span aria-label={t('未保存')} style={{ marginLeft: 4, color: tokens.colorBrandForeground1 }}>●</span>}
                  <span
                    role="button"
                    aria-label={t('{name}を閉じる', { name: tab.label })}
                    className={styles.tabClose}
                    onClick={(e) => {
                      e.stopPropagation();
                      closeTab(tab.key);
                    }}
                  >
                    <Icon.close />
                  </span>
                </span>
              </Tab>
            ))}
          </TabList>
          <div className={styles.grow} />
          {update && (
            <Button
              appearance="subtle"
              onClick={() => void api.updateOpenPage().catch((e) => setNotice(String(e)))}
              title={t('リリースページを開く')}
            >
              {t('新しい版 {tag}', { tag: update.tag })}
            </Button>
          )}
          <Menu>
            <MenuTrigger disableButtonEnhancement>
              <Button appearance="subtle" icon={<Icon.tools />}>
                {t('ツール')}
              </Button>
            </MenuTrigger>
            <MenuPopover>
              <MenuList>
                <MenuItem icon={<Icon.editFile />} onClick={() => openEditor(api.editorOpenLocal())}>
                  {t('この PC のファイルを開く...')}
                </MenuItem>
                <MenuItem icon={<Icon.sshKey />} onClick={() => setKeygenOpen(true)}>
                  {t('SSH鍵を作る...')}
                </MenuItem>
                <MenuItem icon={<Icon.aiModels />} onClick={openAiModels}>
                  {t('AIモデルの管理')}
                </MenuItem>
                <MenuItem icon={<Icon.externalTools />} onClick={() => setExternalOpen(true)}>
                  {t('外部連携（MCP）...')}
                </MenuItem>
                <MenuItem icon={<Icon.portForward />} onClick={() => setForwardOpen(true)}>
                  {t('ポート転送...')}
                </MenuItem>
              </MenuList>
            </MenuPopover>
          </Menu>
          <ToggleButton
            appearance="subtle"
            checked={dockOpen}
            icon={<Icon.dockPanel />}
            aria-label={t('ファイル・AIパネル')}
            onClick={() => setDockOpen((open) => !open)}
          >
            {t('ファイル・AI')}
          </ToggleButton>
          <Tooltip content={t('外観の設定')} relationship="label">
            <Button appearance="subtle" icon={<Icon.settings />} onClick={() => setSettingsOpen(true)} />
          </Tooltip>
        </div>
        <div className={styles.work}>
        <div className={styles.main}>
          {warnings.length > 0 && (
            <div className={styles.banner} style={{ top: startupError ? 56 : 8 }}>
              <MessageBar intent="warning">
                <MessageBarBody>{warnings.join(' / ')}</MessageBarBody>
                <MessageBarActions>
                  <Button size="small" onClick={() => setWarnings([])}>
                    {t('閉じる')}
                  </Button>
                </MessageBarActions>
              </MessageBar>
            </div>
          )}
          {startupError && (
            <div className={styles.banner}>
              <MessageBar intent="error">
                <MessageBarBody>{startupError}</MessageBarBody>
              </MessageBar>
            </div>
          )}
          <div className={mergeClasses(styles.pane, active !== 'home' && styles.paneHidden)}>
            <Home
              profiles={profiles}
              disabled={startupError !== null}
              onConnect={(index) => openTab({ kind: 'ssh', index, label: profiles[index].name || profiles[index].host })}
              onEdit={(index) => {
                setSavedSecrets([]);
                void api.secretsStatus(index).then((s) => setSavedSecrets(s.saved), () => setSavedSecrets([]));
                setDialog({ index, profile: profiles[index], error: null });
              }}
              onNew={() => setDialog({ index: null, profile: emptyProfile(), error: null })}
              onLocal={() => openTab({ kind: 'local', label: 'CMD' })}
              onImport={() => setImportOpen(true)}
            />
          </div>
          {tabs.map((tab) => (
            <div
              key={tab.key}
              className={mergeClasses(styles.pane, styles.paneCol, active !== tab.key && styles.paneHidden)}
            >
              {tab.kind === 'aimodels' ? (
                <ErrorBoundary>
                  <AiModelsPage api={api} />
                </ErrorBoundary>
              ) : tab.kind === 'editor' && tab.doc ? (
                <EditorPane
                  api={api}
                  doc={tab.doc}
                  active={active === tab.key}
                  dark={dark}
                  appearance={appearance}
                  onDirty={(dirty) => setTabs((old) => old.map((t) => (t.key === tab.key ? { ...t, dirty } : t)))}
                  onSaved={(name, path, sessionId) =>
                    setTabs((old) =>
                      old.map((t) =>
                        t.key === tab.key && t.doc ? { ...t, label: name, doc: { ...t.doc, name, path, sessionId } } : t,
                      ),
                    )
                  }
                  onMessage={setNotice}
                />
              ) : (
              <>
              <SnippetBar
                api={api}
                sessionId={tab.sessionId && states[tab.sessionId]?.state === 'connected' ? tab.sessionId : null}
                onError={setNotice}
              />
              <div className={styles.termArea}>
              {(() => {
                const st = tab.sessionId ? states[tab.sessionId] : undefined;
                if (!st || (st.state !== 'error' && st.state !== 'closed')) return null;
                return (
                  <div className={styles.banner}>
                    <MessageBar intent={st.state === 'error' ? 'error' : 'info'}>
                      <MessageBarBody>
                        {st.message || (st.state === 'error' ? t('接続できませんでした') : t('接続が終了しました'))}
                      </MessageBarBody>
                      <MessageBarActions>
                        <Button size="small" onClick={() => closeTab(tab.key)}>
                          {t('タブを閉じる')}
                        </Button>
                      </MessageBarActions>
                    </MessageBar>
                  </div>
                );
              })()}
              <TerminalView
                api={api}
                active={active === tab.key}
                dark={dark}
                appearance={appearance}
                start={(size, sink) =>
                  tab.kind === 'local'
                    ? api.openLocal(size.cols, size.rows, sink)
                    : api.connect(tab.index ?? 0, size.cols, size.rows, sink)
                }
                onOpened={(info) => onOpened(tab.key, info)}
                onError={(message) => {
                  setNotice(message);
                }}
              />
              </div>
              </>
              )}
            </div>
          ))}
        </div>
        {dockOpen && (
          <Grip
            label={t('パネルの幅')}
            onBegin={() => {
              dragStart.current = wide ? dualWidth : dockWidth;
            }}
            onMove={(dx) => {
              // 右へ動かすとパネルが狭くなる。端末側に最低限の幅を残す。
              const max = Math.max(MIN_DOCK, window.innerWidth - MIN_MAIN);
              const next = Math.min(max, Math.max(MIN_DOCK, dragStart.current - dx));
              (wide ? setDualWidth : setDockWidth)(next);
            }}
            onReset={() => (wide ? setDualWidth(DUAL_WIDTH) : setDockWidth(DOCK_WIDTH))}
          />
        )}
        {dockOpen && (
          <aside
            className={styles.dock}
            style={{ width: wide ? dualWidth : dockWidth, maxWidth: `calc(100vw - ${MIN_MAIN}px)` }}
          >
            <TabList
              size="small"
              selectedValue={dockTab}
              onTabSelect={(_, d) => setDockTab(d.value as 'files' | 'ai')}
              style={{ marginBottom: 8 }}
            >
              <Tab value="files">{t('ファイル')}</Tab>
              <Tab value="ai">AI</Tab>
            </TabList>
            <div style={{ height: 'calc(100% - 40px)', minHeight: 0, display: dockTab === 'files' ? 'block' : 'none' }}>
              <FileDock
                api={api}
                sessionId={filesSessionId}
                available={filesSessionId ? (filesState[filesSessionId]?.available ?? false) : false}
                unavailableMessage={filesSessionId ? (filesState[filesSessionId]?.message ?? '') : ''}
                onEdit={(id, path) => openEditor(api.editorOpenRemote(id, path))}
                twoPane={twoPane}
                onTwoPane={setTwoPane}
                paneRatio={paneRatio}
                onPaneRatio={setPaneRatio}
                remoteHidden={remoteHidden}
                onRemoteHidden={setRemoteHidden}
                localHidden={localHidden}
                onLocalHidden={setLocalHidden}
              />
            </div>
            <div style={{ height: 'calc(100% - 40px)', minHeight: 0, display: dockTab === 'ai' ? 'block' : 'none' }}>
              <ErrorBoundary>
                <AiDock api={api} targets={shareTargets} onManage={openAiModels} />
              </ErrorBoundary>
            </div>
          </aside>
        )}
        </div>
        <div className={styles.status}>
          <Caption1>{activeState?.message || notice}</Caption1>
          <div className={styles.grow} />
          <Text size={200}>{activeTab ? activeTab.label : ''}</Text>
          <Button size="small" appearance="transparent" style={{ display: 'none' }} />
        </div>
      </div>
      <ProfileDialog
        open={dialog !== null}
        initial={dialog?.profile ?? emptyProfile()}
        existing={dialog?.index !== null && dialog?.index !== undefined}
        error={dialog?.error ?? null}
        onSave={(p, connect) => void saveProfile(p, connect)}
        onDelete={async () => {
          if (dialog?.index === null || dialog?.index === undefined) return;
          try {
            setProfiles(await api.profileDelete(dialog.index));
            setDialog(null);
            setNotice(t('接続先を削除しました'));
          } catch (error) {
            setDialog({ ...dialog, error: String(error) });
          }
        }}
        onClose={() => setDialog(null)}
        onPickKey={() => api.pickKeyFile()}
        savedSecrets={savedSecrets}
        onForgetSecrets={async () => {
          if (dialog?.index === null || dialog?.index === undefined) return;
          try {
            await api.secretsForget(dialog.index);
            setSavedSecrets([]);
            setNotice(t('保存済みの秘密を削除しました'));
          } catch (error) {
            setDialog({ ...dialog, error: String(error) });
          }
        }}
      />
      <ImportDialog
        api={api}
        open={importOpen}
        onClose={() => setImportOpen(false)}
        onDone={(list, summary) => {
          setProfiles(list);
          setNotice(summary);
        }}
      />
      <SettingsDialog
        api={api}
        open={settingsOpen}
        value={appearance}
        dark={dark}
        onChange={changeAppearance}
        onClose={() => setSettingsOpen(false)}
      />
      <KeygenDialog api={api} open={keygenOpen} registerSession={filesSessionId} onClose={() => setKeygenOpen(false)} />
      <ForwardDialog
        api={api}
        open={forwardOpen}
        sessionId={filesSessionId}
        label={activeTab?.label ?? ''}
        onClose={() => setForwardOpen(false)}
      />
      <ExternalDialog api={api} open={externalOpen} targets={shareTargets} onClose={() => setExternalOpen(false)} />
      <ApprovalDialog
        key={approvals[0]?.id ?? 'none'}
        request={approvals[0] ?? null}
        onAnswer={(allow) => {
          const current = approvals[0];
          if (!current) return;
          setApprovals((old) => old.slice(1));
          void api.approvalReply(current.id, allow).catch((e) => setNotice(String(e)));
        }}
      />
      <Dialog open={discard !== null} onOpenChange={(_, d) => !d.open && setDiscard(null)}>
        <DialogSurface>
          <DialogBody>
            <DialogTitle>{t('保存していない変更があります')}</DialogTitle>
            <DialogContent>
              {t('{name} への変更は保存されていません。閉じると失われます。', { name: tabs.find((tab) => tab.key === discard)?.label ?? '' })}
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setDiscard(null)}>{t('編集に戻る')}</Button>
              <Button
                appearance="primary"
                onClick={() => {
                  if (discard) closeTab(discard, true);
                  setDiscard(null);
                }}
              >
                {t('保存せず閉じる')}
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <Dialog open={quitAsk} onOpenChange={(_, d) => !d.open && setQuitAsk(false)}>
        <DialogSurface>
          <DialogBody>
            <DialogTitle>{t('保存していない編集があります')}</DialogTitle>
            <DialogContent>
              {t('編集中のファイルに保存していない変更があります。このまま終了すると失われます。')}
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setQuitAsk(false)}>{t('やめる')}</Button>
              <Button appearance="primary" onClick={() => void api.appQuit()}>
                {t('保存せず終了')}
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <ChallengeDialog challenge={challenges[0] ?? null} error={challengeError} onAnswer={(a) => void answer(a)} />
    </FluentProvider>
  );
}
