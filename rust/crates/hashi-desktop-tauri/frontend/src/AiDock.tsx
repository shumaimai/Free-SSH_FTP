import { useCallback, useEffect, useRef, useState } from 'react';
import {
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
  Tooltip,
  tokens,
} from '@fluentui/react-components';
import { Attach20Regular, Bot20Regular, ChatAdd20Regular, Dismiss12Regular, Send20Filled, Stop20Regular } from '@fluentui/react-icons';
import type { Api } from './api';
import type { AiAttachment, AiModelsList, AiStatus, ShareMode } from './types';
import { N_, t } from './i18n';

/** 添付の上限(Rust側でも同じ上限で検査する)。 */
const MAX_FILES = 4;
const MAX_BYTES = 5 * 1024 * 1024;

interface Pending extends AiAttachment {
  /** 画像なら縮小表示用のdata URL。 */
  thumb: string | null;
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** ファイルを読み、中身をbase64にする。 */
function readFile(file: File): Promise<Pending> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error(t('「{name}」を読めませんでした', { name: file.name })));
    reader.onload = () => {
      const url = String(reader.result);
      const data = url.slice(url.indexOf(',') + 1);
      const name = file.name || (file.type.startsWith('image/') ? t('貼り付けた画像') : t('貼り付けたファイル'));
      resolve({ name, data, thumb: file.type.startsWith('image/') ? url : null });
    };
    reader.readAsDataURL(file);
  });
}

const useStyles = makeStyles({
  root: { display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, gap: '10px' },
  header: { display: 'flex', flexDirection: 'column', gap: '6px' },
  scroll: { flex: 1, minHeight: 0, overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '10px', paddingRight: '4px' },
  setup: { display: 'flex', flexDirection: 'column', gap: '12px', overflowY: 'auto', height: '100%' },
  row: { display: 'flex', alignItems: 'center', gap: '8px' },
  wrap: { display: 'flex', flexWrap: 'wrap', gap: '6px' },
  grow: { flex: 1, minWidth: 0 },
  share: {
    display: 'flex',
    flexDirection: 'column',
    gap: '6px',
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
  inputRow: { display: 'flex', alignItems: 'flex-end', gap: '6px' },
  chips: { display: 'flex', flexWrap: 'wrap', gap: '6px' },
  chip: {
    display: 'flex',
    alignItems: 'center',
    gap: '4px',
    maxWidth: '100%',
    padding: '2px 4px 2px 6px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground3,
    fontSize: tokens.fontSizeBase200,
  },
  chipName: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', maxWidth: '160px' },
  thumb: { width: '28px', height: '28px', objectFit: 'cover', borderRadius: tokens.borderRadiusSmall },
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

const MODE_LABEL: Record<ShareMode, string> = { consult: N_('読むだけ'), confirm: N_('操作は確認'), auto: N_('自動で操作') };
const MODE_HELP: Record<ShareMode, string> = {
  consult: N_('画面を読むだけで、AIは操作しません。送る前に内容を確認できます。'),
  confirm: N_('AIが操作したいとき、そのつど許可を求めます。'),
  auto: N_('選んだ端末で、確認なしにAIが実行します（停止するまで）。'),
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
  const [files, setFiles] = useState<Pending[]>([]);
  const pendingFiles = useRef<Pending[]>([]);
  const fileInput = useRef<HTMLInputElement>(null);

  const addFiles = (list: File[]) => {
    if (list.length === 0) return;
    const big = list.find((f) => f.size > MAX_BYTES);
    if (big) {
      setMessages((old) => [...old, { role: 'error', text: t('「{name}」が大きすぎます(5MBまで)', { name: big.name }) }]);
      return;
    }
    void Promise.all(list.map(readFile))
      .then((read) =>
        setFiles((old) => {
          const next = [...old, ...read];
          if (next.length > MAX_FILES) {
            setMessages((m) => [...m, { role: 'error', text: t('添付できるのは{n}件までです', { n: MAX_FILES }) }]);
          }
          return next.slice(0, MAX_FILES);
        }),
      )
      .catch((error) => setMessages((old) => [...old, { role: 'error', text: String(error) }]));
  };
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
    setSelected((old) => old.filter((id) => targets.some((x) => x.sessionId === id)));
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
    if ((!text && files.length === 0) || running) return;
    const attached = files;
    setQuestion('');
    setFiles([]);
    const shown = attached.length ? `${text}${text ? '\n' : ''}${t('[添付]')} ${attached.map((f) => f.name).join(', ')}` : text;
    setMessages((old) => [...old, { role: 'user', text: shown }]);
    setRunning(true);
    try {
      const result = await api.aiSend(
        text,
        selected,
        mode,
        attached.map(({ name, data }) => ({ name, data })),
      );
      if (result.preview !== null) {
        // 送る前の確認。会話はまだ始まっていない。
        setRunning(false);
        pendingText.current = shown;
        pendingFiles.current = attached;
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

  if (!status || !models) return <Caption1 className={styles.note}>{t('読み込み中…')}</Caption1>;

  // ---- 登録が1件も無い ----
  if (models.entries.length === 0) {
    return (
      <div className={styles.setup}>
        <Subtitle2>{t('AIモデルが未登録です')}</Subtitle2>
        <Button appearance="primary" style={{ alignSelf: 'flex-start' }} onClick={onManage}>
          {t('AIモデルを登録する')}
        </Button>
      </div>
    );
  }

  // ---- 会話 ----
  const sharing = selected.length > 0;
  const header = (
    <div className={styles.header}>
      <div className={styles.row}>
        <Select
          className={styles.grow}
          aria-label={t('使うAIモデル')}
          value={models.active ?? ''}
          disabled={switching || running}
          onChange={(_, d) => void choose(d.value)}
        >
          {!models.active && <option value="">{t('AIモデルを選んでください')}</option>}
          {models.entries.map((e) => (
            <option key={e.id} value={e.id}>
              {e.label}
            </option>
          ))}
        </Select>
        <Tooltip content={t('AIモデルの管理')} relationship="label">
          <Button appearance="subtle" icon={<Bot20Regular />} onClick={onManage} />
        </Tooltip>
        <Tooltip content={t('新しい会話')} relationship="label">
          <Button
            appearance="subtle"
            icon={<ChatAdd20Regular />}
            disabled={running || !status.connected}
            onClick={() => void api.aiClear().then(() => setMessages([]))}
          />
        </Tooltip>
      </div>
      {switching && <Spinner size="tiny" label={t('切り替え中')} />}
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
        <Caption1 className={styles.note}>{t('使うAIモデルを選んでください。')}</Caption1>
      </div>
    );
  }
  return (
    <div className={styles.root}>
      {header}
      {targets.length > 0 && (
        <div className={styles.share}>
          <div className={styles.wrap}>
            <Caption1 className={styles.note}>{t('見せる端末')}</Caption1>
            {targets.map((target) => (
              <ToggleButton
                key={target.sessionId}
                size="small"
                appearance={selected.includes(target.sessionId) ? 'primary' : 'outline'}
                checked={selected.includes(target.sessionId)}
                onClick={() =>
                  setSelected((old) =>
                    old.includes(target.sessionId) ? old.filter((x) => x !== target.sessionId) : [...old, target.sessionId],
                  )
                }
              >
                {target.label}
              </ToggleButton>
            ))}
          </div>
          {sharing && (
            <>
              <div className={styles.wrap}>
                {(Object.keys(MODE_LABEL) as ShareMode[]).map((m) => (
                  <ToggleButton key={m} size="small" appearance={mode === m ? 'primary' : 'outline'} checked={mode === m} onClick={() => setMode(m)}>
                    {t(MODE_LABEL[m])}
                  </ToggleButton>
                ))}
              </div>
              <Caption1 className={mergeClasses(styles.note, mode === 'auto' && styles.error)}>{t(MODE_HELP[mode])}</Caption1>
            </>
          )}
        </div>
      )}
      <div className={styles.scroll} ref={scrollRef}>
        {messages.length === 0 && <Caption1 className={styles.note}>{t('質問を入力してください。')}</Caption1>}
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
            <Caption1 className={styles.note}>{t('考えています…')}</Caption1>
          </div>
        )}
      </div>
      {files.length > 0 && (
        <div className={styles.chips}>
          {files.map((f, i) => (
            <span key={i} className={styles.chip} title={f.name}>
              {f.thumb && <img className={styles.thumb} src={f.thumb} alt="" />}
              <span className={styles.chipName}>{f.name}</span>
              <Button
                size="small"
                appearance="transparent"
                aria-label={t('{name}を外す', { name: f.name })}
                icon={<Dismiss12Regular />}
                onClick={() => setFiles((old) => old.filter((_, j) => j !== i))}
              />
            </span>
          ))}
        </div>
      )}
      <div className={styles.inputRow}>
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          onChange={(e) => {
            addFiles(Array.from(e.target.files ?? []));
            e.target.value = '';
          }}
        />
        <Tooltip content={t('ファイルを添付(画像・テキスト。Ctrl+Vで貼り付けも可)')} relationship="label">
          <Button
            appearance="subtle"
            icon={<Attach20Regular />}
            disabled={running || files.length >= MAX_FILES}
            onClick={() => fileInput.current?.click()}
          />
        </Tooltip>
        <Textarea
          className={styles.grow}
          resize="none"
          rows={2}
          value={question}
          placeholder={t('質問を入力（Ctrl+Enterで送信）')}
          onChange={(_, d) => setQuestion(d.value)}
          onPaste={(e) => {
            // 画像などのファイルを貼り付けたら添付にする。文字の貼り付けはそのまま。
            const pasted = Array.from(e.clipboardData.files);
            if (pasted.length > 0) {
              e.preventDefault();
              addFiles(pasted);
            }
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
              e.preventDefault();
              void send();
            }
          }}
        />
        {running ? (
          <Tooltip content={t('停止')} relationship="label">
            <Button appearance="outline" icon={<Stop20Regular />} onClick={stop} />
          </Tooltip>
        ) : (
          <Tooltip content={t('送信')} relationship="label">
            <Button appearance="primary" icon={<Send20Filled />} disabled={!question.trim() && files.length === 0} onClick={() => void send()} />
          </Tooltip>
        )}
      </div>
      <PreviewDialog
        mode={mode}
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
          // 表示用の「[添付] …」行は入力欄へ戻さず、添付は添付として戻す。
          const typed = pendingFiles.current.length ? text.replace(new RegExp(`\\n?${escapeRegExp(t('[添付]'))} [^\\n]*$`), '') : text;
          setQuestion((current) => current || typed);
          setFiles((current) => (current.length ? current : pendingFiles.current));
        }}
      />
    </div>
  );
}

function PreviewDialog({
  mode,
  context,
  onSend,
  onCancel,
}: {
  mode: ShareMode;
  context: string | null;
  onSend(c: string): void;
  onCancel(): void;
}) {
  const styles = useStyles();
  const [text, setText] = useState('');
  useEffect(() => setText(context ?? ''), [context]);
  return (
    <Dialog open={context !== null} modalType="alert">
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 680 }}>
        <DialogBody>
          <DialogTitle>{t('AIへ送る内容の確認')}</DialogTitle>
          <DialogContent>
            <Caption1>{t('端末の画面がそのままAIに送られます。秘密情報が残っていないか確認し、必要なら直してください。')}</Caption1>
            {mode === 'consult' ? (
              <Caption1>{t('「読むだけ」では、AIが見るのはここで送る内容だけです。')}</Caption1>
            ) : (
              <Caption1 className={styles.error}>
                {t('操作を任せるモードでは、AIはこのあとも共有中の端末の出力を読み取ります(秘密らしい文字列は伏せますが、完全ではありません)。')}
              </Caption1>
            )}
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
            <Button onClick={onCancel}>{t('戻る')}</Button>
            <Button appearance="primary" onClick={() => onSend(text)}>
              {t('この内容を送る')}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
