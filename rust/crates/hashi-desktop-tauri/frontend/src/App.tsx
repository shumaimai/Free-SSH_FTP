import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Button,
  Caption1,
  FluentProvider,
  makeStyles,
  mergeClasses,
  MessageBar,
  MessageBarActions,
  MessageBarBody,
  Tab,
  TabList,
  Text,
  tokens,
  webDarkTheme,
  webLightTheme,
} from '@fluentui/react-components';
import { Dismiss12Regular, Home20Regular, WindowConsole20Regular, PlugConnected20Regular } from '@fluentui/react-icons';
import type { Api } from './api';
import type { Challenge, ChallengeAnswer, Profile, SessionInfo, SessionState } from './types';
import { emptyProfile } from './types';
import { Home } from './Home';
import { ProfileDialog } from './ProfileDialog';
import { ChallengeDialog } from './ChallengeDialog';
import { TerminalView } from './TerminalView';

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
  main: { position: 'relative', minHeight: 0, backgroundColor: tokens.colorNeutralBackground2 },
  pane: { position: 'absolute', inset: 0 },
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
  kind: 'local' | 'ssh';
  index?: number;
  label: string;
  sessionId?: string;
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
  const dark = useDark();
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [startupError, setStartupError] = useState<string | null>(null);
  const [tabs, setTabs] = useState<TabInfo[]>([]);
  const [active, setActive] = useState<string>('home');
  const [states, setStates] = useState<Record<string, { state: SessionState; message: string }>>({});
  const [notice, setNotice] = useState('');
  const [dialog, setDialog] = useState<{ index: number | null; profile: Profile; error: string | null } | null>(null);
  const [challenges, setChallenges] = useState<Challenge[]>([]);
  const [challengeError, setChallengeError] = useState<string | null>(null);
  const closed = useRef(new Set<string>());

  useEffect(() => {
    void api.profilesList().then((r) => {
      setProfiles(r.profiles);
      setStartupError(r.error);
    });
    const subscriptions: Array<Promise<() => void>> = [
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

  const openTab = useCallback((tab: Omit<TabInfo, 'key'>) => {
    const key = crypto.randomUUID();
    setTabs((old) => [...old, { ...tab, key }]);
    setActive(key);
  }, []);

  const closeTab = useCallback(
    (key: string) => {
      const tab = tabs.find((t) => t.key === key);
      closed.current.add(key);
      setTabs((old) => old.filter((t) => t.key !== key));
      setActive((current) => (current === key ? 'home' : current));
      if (tab?.sessionId) void api.close(tab.sessionId).catch((e) => setNotice(String(e)));
    },
    [api, tabs],
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
      setNotice('保存しました');
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
      if (message.includes('終了')) {
        setChallenges((old) => old.filter((c) => c.challenge_id !== current.challenge_id));
      }
    }
  };

  const activeTab = tabs.find((t) => t.key === active);
  const activeState = activeTab?.sessionId ? states[activeTab.sessionId] : undefined;
  const dotColor = (tab: TabInfo) => {
    const s = tab.sessionId ? states[tab.sessionId]?.state : undefined;
    if (s === 'connected') return tokens.colorPaletteGreenForeground1;
    if (s === 'error') return tokens.colorPaletteRedForeground1;
    if (s === 'closed') return tokens.colorNeutralForeground4;
    return tokens.colorPaletteYellowForeground1;
  };

  return (
    <FluentProvider theme={dark ? webDarkTheme : webLightTheme} style={{ height: '100vh' }}>
      <div className={styles.shell}>
        <div className={styles.bar}>
          <span className={styles.brand}>Hashi</span>
          <TabList
            selectedValue={active}
            onTabSelect={(_, d) => setActive(String(d.value))}
            size="medium"
          >
            <Tab value="home" icon={<Home20Regular />}>
              ホーム
            </Tab>
            {tabs.map((tab) => (
              <Tab
                key={tab.key}
                value={tab.key}
                icon={tab.kind === 'ssh' ? <PlugConnected20Regular /> : <WindowConsole20Regular />}
              >
                <span style={{ display: 'inline-flex', alignItems: 'center' }}>
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: 4,
                      background: dotColor(tab),
                      marginRight: 8,
                      display: 'inline-block',
                    }}
                  />
                  {tab.label}
                  <span
                    role="button"
                    aria-label={`${tab.label}を閉じる`}
                    className={styles.tabClose}
                    onClick={(e) => {
                      e.stopPropagation();
                      closeTab(tab.key);
                    }}
                  >
                    <Dismiss12Regular />
                  </span>
                </span>
              </Tab>
            ))}
          </TabList>
          <div className={styles.grow} />
        </div>
        <div className={styles.main}>
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
              onEdit={(index) => setDialog({ index, profile: profiles[index], error: null })}
              onNew={() => setDialog({ index: null, profile: emptyProfile(), error: null })}
              onLocal={() => openTab({ kind: 'local', label: 'CMD' })}
            />
          </div>
          {tabs.map((tab) => (
            <div key={tab.key} className={mergeClasses(styles.pane, active !== tab.key && styles.paneHidden)}>
              {(() => {
                const st = tab.sessionId ? states[tab.sessionId] : undefined;
                if (!st || (st.state !== 'error' && st.state !== 'closed')) return null;
                return (
                  <div className={styles.banner}>
                    <MessageBar intent={st.state === 'error' ? 'error' : 'info'}>
                      <MessageBarBody>
                        {st.message || (st.state === 'error' ? '接続できませんでした' : '接続が終了しました')}
                      </MessageBarBody>
                      <MessageBarActions>
                        <Button size="small" onClick={() => closeTab(tab.key)}>
                          タブを閉じる
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
          ))}
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
            setNotice('接続先を削除しました');
          } catch (error) {
            setDialog({ ...dialog, error: String(error) });
          }
        }}
        onClose={() => setDialog(null)}
        onPickKey={() => api.pickKeyFile()}
      />
      <ChallengeDialog challenge={challenges[0] ?? null} error={challengeError} onAnswer={(a) => void answer(a)} />
    </FluentProvider>
  );
}
