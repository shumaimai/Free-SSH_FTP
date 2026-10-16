import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
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
  Link,
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
import { Delete20Regular, Send20Filled, Stop20Regular, Settings20Regular } from '@fluentui/react-icons';
import type { Api } from './api';
import type { AiConnectResult, AiServiceInfo, AiStatus, ShareMode } from './types';

const useStyles = makeStyles({
  root: { display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, gap: '10px' },
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

export function AiDock({ api, targets }: Props) {
  const styles = useStyles();
  const [services, setServices] = useState<AiServiceInfo[]>([]);
  const [protocols, setProtocols] = useState<[string, string][]>([]);
  const [status, setStatus] = useState<AiStatus | null>(null);
  const [setup, setSetup] = useState(false);

  // 設定画面の入力
  const [service, setService] = useState('openai');
  const [base, setBase] = useState('');
  const [key, setKey] = useState('');
  const [model, setModel] = useState('');
  const [protocol, setProtocol] = useState('auto');
  const [useTools, setUseTools] = useState(true);
  const [persist, setPersist] = useState(true);
  const [hasKey, setHasKey] = useState(false);
  const [busy, setBusy] = useState(false);
  const [needModel, setNeedModel] = useState(false);
  const [candidates, setCandidates] = useState<string[]>([]);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  // 会話
  const [messages, setMessages] = useState<Msg[]>([]);
  const [question, setQuestion] = useState('');
  const [running, setRunning] = useState(false);
  const [selected, setSelected] = useState<string[]>([]);
  const [mode, setMode] = useState<ShareMode>('consult');
  const [preview, setPreview] = useState<string | null>(null);
  const endRef = useRef<HTMLDivElement>(null);

  const current = useMemo(() => services.find((s) => s.id === service), [services, service]);
  const isUrlService = service === 'local' || service === 'custom';

  const refresh = useCallback(async () => {
    const s = await api.aiStatus();
    setStatus(s);
    // 接続済みでも、設定を開き直したときに今のサービスが選ばれているよう、前回の設定を入れておく。
    const saved = s.saved;
    if (saved?.service) setService(saved.service);
    if (saved?.base !== undefined) setBase(saved.base);
    if (saved?.protocol) setProtocol(saved.protocol);
    if (saved?.use_tools !== undefined) setUseTools(saved.use_tools);
    setSetup(!s.connected);
    return s;
  }, [api]);

  useEffect(() => {
    void api.aiServices().then(setServices);
    void api.aiProtocols().then(setProtocols);
    void refresh();
    const subs = [
      api.onAiRestored(() => void refresh()),
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

  // サービスを選んだとき、保存済みのキーがあるか確かめる（キー自体は受け取らない）。
  useEffect(() => {
    if (!setup || !current) return;
    setKey('');
    setModel('');
    setCandidates([]);
    setNeedModel(false);
    setMessage(null);
    if (!isUrlService) setBase('');
    void api.aiHasKey(service, base).then(setHasKey).catch(() => setHasKey(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [service, setup]);

  // 接続先URLを変えたら、そのURLの保存キーの有無を見直す（入力済みのキーは消さない）。
  useEffect(() => {
    if (!setup || !isUrlService) return;
    const timer = setTimeout(() => {
      void api.aiHasKey(service, base).then(setHasKey).catch(() => setHasKey(false));
    }, 400);
    return () => clearTimeout(timer);
  }, [api, base, service, setup, isUrlService]);

  useEffect(() => endRef.current?.scrollIntoView({ block: 'end' }), [messages, running]);
  // 終わった端末は共有対象から外す。
  useEffect(() => {
    setSelected((old) => old.filter((id) => targets.some((t) => t.sessionId === id)));
  }, [targets]);

  const connect = async () => {
    setBusy(true);
    setMessage(null);
    try {
      const result: AiConnectResult = await api.aiConnect({
        service,
        base,
        key,
        model,
        protocol,
        useTools,
        persist,
      });
      if (result.kind === 'need_model') {
        setNeedModel(true);
        setCandidates(result.models);
        setMessage({ text: result.message, error: true });
      } else {
        setKey('');
        setMessages([]);
        await refresh();
      }
    } catch (error) {
      setMessage({ text: String(error), error: true });
    } finally {
      setBusy(false);
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

  // ---- 設定画面 ----
  if (setup || !status?.connected) {
    return (
      <div className={styles.setup}>
        <Subtitle2>AIをつなぐ</Subtitle2>
        <Caption1>サービスを選び、APIキーを貼り付けるだけです。キーはこのPCの専用の暗号化ファイルにだけ保存します。</Caption1>
        <Field label="1  サービス">
          <div className={styles.wrap}>
            {services.map((s) => (
              <ToggleButton key={s.id} appearance={service === s.id ? 'primary' : 'outline'} checked={service === s.id} disabled={busy} onClick={() => setService(s.id)}>
                {s.label}
              </ToggleButton>
            ))}
          </div>
        </Field>
        {current && <Caption1>{current.blurb}</Caption1>}
        {isUrlService && (
          <Field label="接続先URL">
            <Input
              value={base}
              disabled={busy}
              placeholder={current?.defaultBase || 'https://example.com/v1'}
              onChange={(_, d) => {
                // キーは接続を押したときの接続先へだけ送る。先にキーを貼っても消さない。
                setBase(d.value);
                setNeedModel(false);
              }}
            />
          </Field>
        )}
        {current?.needsKey && (
          <>
            <Field label="2  APIキー">
              <Input
                type="password"
                value={key}
                disabled={busy}
                placeholder={hasKey ? '保存済みのキーを使います（変えるときだけ貼り付け）' : 'ここに貼り付け'}
                onChange={(_, d) => setKey(d.value)}
              />
            </Field>
            <div className={styles.row}>
              <Checkbox checked={persist} onChange={(_, d) => setPersist(d.checked === true)} label="このPCに保存する" />
              <div className={styles.grow} />
              {current.hasKeyPage && (
                <Link onClick={() => void api.aiOpenKeyPage(service).catch((e) => setMessage({ text: String(e), error: true }))}>
                  キーの取得ページを開く
                </Link>
              )}
            </div>
          </>
        )}
        <Field label={needModel ? 'モデル名（選んでください）' : 'モデル名（空なら自動で選びます）'}>
          <div className={styles.row}>
            <Input
              className={styles.grow}
              value={model}
              disabled={busy}
              placeholder="例: claude-sonnet-5-5 / gpt-5"
              onChange={(_, d) => setModel(d.value)}
            />
            {candidates.length > 0 && (
              <Select value="" disabled={busy} onChange={(_, d) => d.value && setModel(d.value)} style={{ maxWidth: 150 }}>
                <option value="">一覧から選ぶ</option>
                {candidates.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </Select>
            )}
          </div>
        </Field>
        {isUrlService && (
          <Accordion collapsible>
            <AccordionItem value="adv">
              <AccordionHeader>詳細設定</AccordionHeader>
              <AccordionPanel>
                <Field label="通信の形式">
                  <Select value={protocol} onChange={(_, d) => setProtocol(d.value)}>
                    {protocols.map(([id, label]) => (
                      <option key={id} value={id}>
                        {label}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Checkbox
                  checked={useTools}
                  onChange={(_, d) => setUseTools(d.checked === true)}
                  label="このAIは関数呼び出し（ツール）に対応している"
                />
                <Caption1 className={styles.note}>対応していないAIでは外してください。端末の閲覧だけになります。</Caption1>
              </AccordionPanel>
            </AccordionItem>
          </Accordion>
        )}
        <div className={styles.row}>
          <Button appearance="primary" disabled={busy || (current?.needsKey && !key && !hasKey) || (isUrlService && !base && service === 'custom')} onClick={() => void connect()}>
            接続する
          </Button>
          {busy && <Spinner size="tiny" />}
          {status?.connected && (
            <Button disabled={busy} onClick={() => setSetup(false)}>
              会話へ戻る
            </Button>
          )}
        </div>
        {message && (
          <MessageBar intent={message.error ? 'error' : 'info'}>
            <MessageBarBody>{message.text}</MessageBarBody>
          </MessageBar>
        )}
        <Caption1 className={styles.note}>接続時に短い確認の質問を1回送ります。利用料は提供元の料金に従います。</Caption1>
        {current?.needsKey && hasKey && (
          <Button
            appearance="subtle"
            style={{ alignSelf: 'flex-start', color: tokens.colorPaletteRedForeground1 }}
            disabled={busy}
            onClick={() =>
              void api.aiForget(service, base).then(() => {
                setHasKey(false);
                setMessage({ text: '保存したキーを削除しました。', error: false });
              })
            }
          >
            このサービスの保存キーを削除
          </Button>
        )}
      </div>
    );
  }

  // ---- 会話 ----
  const sharing = selected.length > 0;
  return (
    <div className={styles.root}>
      <div className={styles.row}>
        <Select
          className={styles.grow}
          aria-label="モデル"
          value={status.model ?? ''}
          onChange={(_, d) => void api.aiSwitchModel(d.value).then(async () => {
            setMessages([{ role: 'note', text: `モデルを ${d.value} に切り替えました。会話は新しくなっています。` }]);
            await refresh();
          })}
        >
          {[...new Set([status.model ?? '', ...status.models])].filter(Boolean).map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </Select>
        <Button appearance="subtle" aria-label="接続設定" icon={<Settings20Regular />} onClick={() => setSetup(true)} />
        <Button
          appearance="subtle"
          aria-label="会話を消去"
          icon={<Delete20Regular />}
          disabled={running}
          onClick={() => void api.aiClear().then(() => setMessages([]))}
        />
      </div>
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
      <div className={styles.scroll}>
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
        <div ref={endRef} />
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
          setMessages((old) => [...old, { role: 'note', text: '送信を取りやめました。' }]);
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
