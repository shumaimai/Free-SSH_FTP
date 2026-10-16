import { useCallback, useEffect, useRef, useState } from 'react';
import {
  Body1,
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  makeStyles,
  mergeClasses,
  MessageBar,
  MessageBarBody,
  Select,
  Spinner,
  Subtitle2,
  Textarea,
  ToggleButton,
  tokens,
} from '@fluentui/react-components';
import { Bot20Regular, ChatAdd20Regular, Send20Filled, Stop20Regular } from '@fluentui/react-icons';
import type { Api } from './api';
import type { AiModelsList, AiStatus, ShareMode } from './types';

const useStyles = makeStyles({
  root: { display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, gap: '10px' },
  header: { display: 'flex', flexDirection: 'column', gap: '4px' },
  scroll: { flex: 1, minHeight: 0, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '10px', paddingRight: '4px' },
  setup: { display: 'flex', flexDirection: 'column', gap: '12px', overflowY: 'auto', height: '100%' },
  row: { display: 'flex', alignItems: 'center', gap: '8px' },
  wrap: { display: 'flex', flexWrap: 'wrap', gap: '6px' },
  grow: { flex: 1, minWidth: 0 },
  share: {
    padding: '10px 12px',
    borderRadius: tokens.borderRadiusLarge,
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    display: 'flex',
    flexDirection: 'column',
    gap: '8px',
  },
  bubbleUser: {
    alignSelf: 'flex-end',
    maxWidth: '88%',
    padding: '8px 12px',
    borderRadius: tokens.borderRadiusLarge,
    backgroundColor: tokens.colorBrandBackground2,
    whiteSpace: 'pre-wrap',
    overflowWrap: 'anywhere',
  },
  bubbleAi: {
    alignSelf: 'stretch',
    padding: '10px 12px',
    borderRadius: tokens.borderRadiusLarge,
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    overflowWrap: 'anywhere',
  },
  para: { whiteSpace: 'pre-wrap', margin: 0 },
  code: {
    fontFamily: 'Consolas, "Cascadia Mono", monospace',
    fontSize: '13px',
    margin: '6px 0',
    padding: '8px 10px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground3,
    whiteSpace: 'pre-wrap',
    overflowX: 'auto',
  },
  error: { color: tokens.colorPaletteRedForeground1, whiteSpace: 'pre-wrap' },
  note: { color: tokens.colorNeutralForeground3 },
  examples: { display: 'flex', flexDirection: 'column', gap: '6px', alignItems: 'flex-start' },
  mono: { fontFamily: 'Consolas, "Cascadia Mono", monospace', fontSize: '13px' },
});

export interface ShareTarget {
  sessionId: string;
  label: string;
}

interface Props {
  api: Api;
  targets: ShareTarget[];
  /** AIモデルの管理ページを開く。 */
  onManage(): void;
}

interface Msg {
  role: 'user' | 'ai' | 'note' | 'error';
  text: string;
}

const MODE_LABEL: Record<ShareMode, string> = { consult: '読むだけ', confirm: '操作は確認', auto: '自動で操作' };
const MODE_HELP: Record<ShareMode, string> = {
  consult: '画面を読むだけで、AIは操作しません。送る前に内容を確認できます。',
  confirm: 'AIが操作したいとき、そのつど許可を求めます。',
  auto: '選んだ端末で、確認なしにAIが実行します（15分間）。',
};

/** 本文を```で分け、コードの部分だけ等幅で出す。 */
function Rich({ text }: { text: string }) {
  const styles = useStyles();
  const parts = text.split('```');
  return (
    <>
      {parts.map((part, i) => {
        if (i % 2 === 1) {
          // 先頭行は言語名なので除く。
          const nl = part.indexOf('\n');
          return (
            <pre key={i} className={styles.code}>
              {(nl >= 0 ? part.slice(nl + 1) : part).trimEnd()}
            </pre>
          );
        }
        return part.trim() === '' ? null : (
          <p key={i} className={styles.para}>
            {part.trim()}
          </p>
        );
      })}
    </>
  );
}

export function AiDock({ api, targets, onManage }: Props) {
  const styles = useStyles();
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [models, setModels] = useState<AiModelsList | null>(null);
  const [switching, setSwitching] = useState(false);
  const [problem, setProblem] = useState('');

  // 会話
  const [messages, setMessages] = useState<Msg[]>([]);
  const [question, setQuestion] = useState('');
  const [running, setRunning] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [mode, setMode] = useState<ShareMode>('consult');
  const [preview, setPreview] = useState<string | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  // 確認窓で「戻る」を押したとき、入力欄へ戻す質問。
  const pendingText = useRef('');
  // 使用中のモデルが変わったら、前の会話の表示は引き継がない。
  const shownModel = useRef<string | null>(null);

  const refresh = useCallback(async () => {
    const [s, list] = await Promise.all([api.aiStatus(), api.aiModelsList()]);
    setStatus(s);
    setModels(list);
    const key = s.connected ? `${list.active ?? ''}:${s.model ?? ''}` : null;
    if (shownModel.current !== null && shownModel.current !== key) setMessages([]);
    shownModel.current = key;
    return s;
  }, [api]);

  useEffect(() => {
    void refresh();
    const subs = [
      api.onAiRestored(() => void refresh()),
      api.onAiModelsChanged(() => void refresh()),
      api.onAiDelta((text) =>
        setMessages((old) => {
          const last = old[old.length - 1];
          // 送信の応答より先に届いた出力も取りこぼさない。
          if (last?.role !== 'ai') return [...old, { role: 'ai' as const, text }];
          return [...old.slice(0, -1), { ...last, text: last.text + text }];
        }),
      ),
      api.onAiDone(({ error, text }) => {
        setRunning(false);
        setMessages((old) => {
          const last = old[old.length - 1];
          const copy = last?.role === 'ai' ? old.slice(0, -1) : old;
          if (error) return [...copy, ...(last?.role === 'ai' && last.text ? [last] : []), { role: 'error' as const, text: error }];
          // ストリームで届かなかった場合は、完成した本文を出す。
          return last?.role === 'ai' && last.text ? old : [...copy, { role: 'ai' as const, text }];
        });
      }),
    ];
    return () => subs.forEach((p) => void p.then((off) => off()));
  }, [api, refresh]);

  // 会話欄だけを末尾へ送る。scrollIntoViewは外側の枠（アプリ全体）まで動かし、画面がずれることがある。
  useEffect(() => {
    const box = scrollRef.current;
    if (box) box.scrollTop = box.scrollHeight;
  }, [messages, running]);
  // 終わった端末は共有対象から外す。
  useEffect(() => {
    setSelected((old) => old.filter((id) => targets.some((t) => t.sessionId === id)));
  }, [targets]);

  const choose = async (id: string) => {
    if (!id || id === models?.active || switching) return;
    setSwitching(true);
    setProblem('');
    try {
      await api.aiModelUse(id);
      await refresh();
    } catch (error) {
      setProblem(String(error));
    } finally {
      setSwitching(false);
    }
  };

  // 応答のイベント（ai-delta / ai-done）は、命令の戻りより先に届くことがある。
  // 「実行中」は命令を出す前に立て、終わりの通知（ai-done）だけが下ろす。後から立て直すと下ろせなくなる。
  const abort = (error: unknown) => {
    setRunning(false);
    setMessages((old) => [...old, { role: 'error', text: String(error) }]);
  };

  const send = async () => {
    const text = question.trim();
    if (!text || running) return;
    setQuestion('');
    setMessages((old) => [...old, { role: 'user', text }]);
    setRunning(true);
    try {
      const result = await api.aiSend(text, selected, mode);
      if (result.preview !== null) {
        // 送る前の確認。会話はまだ始まっていない。
        setRunning(false);
        pendingText.current = text;
        setPreview(result.preview);
      }
    } catch (error) {
      abort(error);
    }
  };

  const confirm = async (context: string) => {
    setPreview(null);
    setRunning(true);
    try {
      await api.aiConfirm(context);
    } catch (error) {
      abort(error);
    }
  };

  const stop = () => {
    void api.aiStop();
  };

  if (!status || !models) return <Caption1 className={styles.note}>読み込み中…</Caption1>;

  // ---- 登録が1件も無い ----
  if (models.entries.length === 0) {
    return (
      <div className={styles.setup}>
        <Subtitle2>AIをつなぐ</Subtitle2>
        <Caption1>
          使いたいAIモデルを登録すると、ここで相談できます。登録・変更・削除は「AIモデルの管理」で行います。
        </Caption1>
        <Button appearance="primary" style={{ alignSelf: 'flex-start' }} onClick={onManage}>
          AIモデルを登録する
        </Button>
      </div>
    );
  }

  // ---- 会話 ----
  const sharing = selected.length > 0;
  const header = (
    <div className={styles.header}>
      <Select
        aria-label="使うAIモデル"
        value={models.active ?? ''}
        disabled={switching || running}
        onChange={(_, d) => void choose(d.value)}
      >
        {!models.active && <option value="">AIモデルを選んでください</option>}
        {models.entries.map((e) => (
          <option key={e.id} value={e.id}>
            {e.label}
          </option>
        ))}
      </Select>
      <div className={styles.row}>
        <Button size="small" appearance="subtle" icon={<Bot20Regular />} onClick={onManage}>
          AIモデルの管理
        </Button>
        <div className={styles.grow} />
        <Button
          size="small"
          appearance="subtle"
          icon={<ChatAdd20Regular />}
          disabled={running || !status.connected}
          onClick={() => void api.aiClear().then(() => setMessages([]))}
        >
          新しい会話
        </Button>
      </div>
      {switching && <Spinner size="tiny" label="切り替え中" />}
      {problem && (
        <MessageBar intent="error">
          <MessageBarBody>{problem}</MessageBarBody>
        </MessageBar>
      )}
      {models.warning && (
        <MessageBar intent="warning">
          <MessageBarBody>{models.warning}</MessageBarBody>
        </MessageBar>
      )}
    </div>
  );
  if (!status.connected) {
    return (
      <div className={styles.root}>
        {header}
        <Body1 className={styles.note}>使うAIモデルを上で選ぶと、ここで相談できます。</Body1>
      </div>
    );
  }
  return (
    <div className={styles.root}>
      {header}
      <div className={styles.share}>
        <Caption1>AIに見せる端末</Caption1>
        <div className={styles.wrap}>
          {targets.length === 0 && <Caption1 className={styles.note}>（なし）端末を開くと選べます</Caption1>}
          {targets.map((t) => (
            <ToggleButton
              key={t.sessionId}
              size="small"
              appearance={selected.includes(t.sessionId) ? 'primary' : 'outline'}
              checked={selected.includes(t.sessionId)}
              onClick={() =>
                setSelected((old) => (old.includes(t.sessionId) ? old.filter((x) => x !== t.sessionId) : [...old, t.sessionId]))
              }
            >
              {t.label}
            </ToggleButton>
          ))}
        </div>
        <div className={styles.wrap}>
          {(Object.keys(MODE_LABEL) as ShareMode[]).map((m) => (
            <ToggleButton key={m} size="small" appearance={mode === m ? 'primary' : 'outline'} disabled={!sharing} checked={mode === m} onClick={() => setMode(m)}>
              {MODE_LABEL[m]}
            </ToggleButton>
          ))}
        </div>
        <Caption1 className={mergeClasses(styles.note, sharing && mode === 'auto' && styles.error)}>
          {sharing ? MODE_HELP[mode] : '端末は見せていません。質問だけがAIに届きます。'}
        </Caption1>
      </div>
      <div className={styles.scroll} ref={scrollRef}>
        {messages.length === 0 && (
          <div className={styles.examples}>
            <Body1 className={styles.note}>何でも質問できます。上で端末を選ぶと、その画面を見せて相談できます。</Body1>
            {['この画面のエラーの原因を教えて', 'いまの出力を短く要約して', '次に打つコマンドを提案して'].map((example) => (
              <Button key={example} size="small" onClick={() => setQuestion(example)}>
                {example}
              </Button>
            ))}
          </div>
        )}
        {messages.map((m, i) =>
          m.role === 'user' ? (
            <div key={i} className={styles.bubbleUser}>
              {m.text}
            </div>
          ) : m.role === 'ai' ? (
            <div key={i} className={styles.bubbleAi}>
              {m.text ? <Rich text={m.text} /> : <Caption1 className={styles.note}>…</Caption1>}
            </div>
          ) : (
            <Caption1 key={i} className={m.role === 'error' ? styles.error : styles.note}>
              {m.text}
            </Caption1>
          ),
        )}
        {running && messages[messages.length - 1]?.role !== 'ai' && (
          <div className={styles.bubbleAi}>
            <Caption1 className={styles.note}>考えています…</Caption1>
          </div>
        )}
      </div>
      <Textarea
        resize="none"
        rows={3}
        value={question}
        placeholder="質問や、端末への依頼を入力（Ctrl+Enterで送信）"
        onChange={(_, d) => setQuestion(d.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <div className={styles.row}>
        {running ? (
          <>
            <Button appearance="outline" icon={<Stop20Regular />} onClick={stop}>
              停止
            </Button>
            <Spinner size="tiny" />
          </>
        ) : (
          <Button appearance="primary" icon={<Send20Filled />} disabled={!question.trim()} onClick={() => void send()}>
            送信
          </Button>
        )}
      </div>
      <PreviewDialog
        context={preview}
        onSend={(c) => void confirm(c)}
        onCancel={() => {
          setPreview(null);
          void api.aiCancelPreview();
          // 送らなかった質問は会話から外し、入力欄へ戻す。
          const text = pendingText.current;
          setMessages((old) => {
            const last = old[old.length - 1];
            return last?.role === 'user' && last.text === text ? old.slice(0, -1) : old;
          });
          setQuestion((current) => current || text);
        }}
      />
    </div>
  );
}

function PreviewDialog({ context, onSend, onCancel }: { context: string | null; onSend(c: string): void; onCancel(): void }) {
  const styles = useStyles();
  const [text, setText] = useState('');
  useEffect(() => setText(context ?? ''), [context]);
  return (
    <Dialog open={context !== null} modalType="alert">
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 680 }}>
        <DialogBody>
          <DialogTitle>AIへ送る内容の確認</DialogTitle>
          <DialogContent>
            <Caption1>端末の画面がそのままAIに送られます。秘密情報が残っていないか確認し、必要なら直してください。</Caption1>
            <Textarea
              textarea={{ className: styles.mono }}
              style={{ width: '100%', marginTop: 8 }}
              resize="vertical"
              rows={14}
              value={text}
              onChange={(_, d) => setText(d.value)}
            />
          </DialogContent>
          <DialogActions>
            <Button onClick={onCancel}>戻る</Button>
            <Button appearance="primary" onClick={() => onSend(text)}>
              この内容を送る
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
