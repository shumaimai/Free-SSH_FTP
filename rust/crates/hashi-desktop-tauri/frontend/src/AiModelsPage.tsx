// AIモデルの管理ページ。登録の一覧と、追加・変更・削除をここで行う。
// AIドックでは登録したモデルを選ぶだけにし、設定ダイアログ（外観など）とは分けている。
import { useCallback, useEffect, useState } from 'react';
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Badge,
  Button,
  Caption1,
  Checkbox,
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
  Text,
  Title2,
  tokens,
} from '@fluentui/react-components';
import { Add20Regular, Checkmark20Regular } from '@fluentui/react-icons';
import type { Api } from './api';
import { emptyAiModel, type AiModelEntry, type AiModelView, type AiModelsList, type AiServiceInfo } from './types';

const useStyles = makeStyles({
  page: { display: 'flex', flexDirection: 'column', gap: '12px', padding: '24px 32px', height: '100%', boxSizing: 'border-box', overflow: 'hidden' },
  head: { display: 'flex', flexDirection: 'column', gap: '4px' },
  body: { display: 'grid', gridTemplateColumns: 'minmax(240px, 320px) minmax(0, 1fr)', gap: '20px', flex: 1, minHeight: 0 },
  list: { display: 'flex', flexDirection: 'column', gap: '8px', minHeight: 0, overflowY: 'auto', paddingRight: '4px' },
  item: {
    display: 'flex',
    flexDirection: 'column',
    gap: '4px',
    textAlign: 'left',
    padding: '10px 12px',
    borderRadius: tokens.borderRadiusLarge,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    backgroundColor: tokens.colorNeutralBackground1,
    color: 'inherit',
    font: 'inherit',
    cursor: 'pointer',
    ':hover': { backgroundColor: tokens.colorNeutralBackground1Hover },
  },
  itemSelected: { border: `1px solid ${tokens.colorBrandStroke1}`, backgroundColor: tokens.colorBrandBackground2 },
  itemRow: { display: 'flex', alignItems: 'center', gap: '6px', flexWrap: 'wrap' },
  panel: {
    display: 'flex',
    flexDirection: 'column',
    gap: '10px',
    minHeight: 0,
    overflowY: 'auto',
    padding: '16px 20px',
    borderRadius: tokens.borderRadiusLarge,
    backgroundColor: tokens.colorNeutralBackground1,
    border: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  wrap: { display: 'flex', flexWrap: 'wrap', gap: '6px' },
  row: { display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' },
  grow: { flex: 1, minWidth: 0 },
  actions: { display: 'flex', alignItems: 'center', gap: '8px', flexWrap: 'wrap' },
  note: { color: tokens.colorNeutralForeground3 },
  empty: { padding: '24px', textAlign: 'center' },
});

interface Props {
  api: Api;
}

type Selection = string | 'new' | null;

function sameEntry(a: AiModelEntry, b: AiModelEntry): boolean {
  return (
    a.name === b.name &&
    a.service === b.service &&
    a.base === b.base &&
    a.model === b.model &&
    a.protocol === b.protocol &&
    a.useTools === b.useTools
  );
}

export function AiModelsPage({ api }: Props) {
  const styles = useStyles();
  const [services, setServices] = useState<AiServiceInfo[]>([]);
  const [protocols, setProtocols] = useState<[string, string][]>([]);
  const [list, setList] = useState<AiModelsList | null>(null);
  const [selection, setSelection] = useState<Selection>(null);
  const [draft, setDraft] = useState<AiModelEntry>(emptyAiModel());
  const [key, setKey] = useState('');
  const [candidates, setCandidates] = useState<string[]>([]);
  const [busy, setBusy] = useState('');
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const [armed, setArmed] = useState<'' | 'model' | 'key'>('');
  // 今のサービス・URLにキーが保存済みか（同じ接続先の他のモデルと共有するため、登録とは別に調べる）。
  const [savedKey, setSavedKey] = useState(false);

  const entries = list?.entries ?? [];
  const current: AiModelView | undefined = typeof selection === 'string' ? entries.find((e) => e.id === selection) : undefined;
  const service = services.find((s) => s.id === draft.service);
  const isUrl = draft.service === 'local' || draft.service === 'custom';
  const dirty = selection === 'new' ? true : current ? !sameEntry(current, draft) || key !== '' : false;

  const open = useCallback((entry: AiModelEntry | null) => {
    setDraft(entry ? { ...entry } : emptyAiModel());
    setKey('');
    setCandidates([]);
    setMessage(null);
    setArmed('');
  }, []);

  const reload = useCallback(async () => {
    const next = await api.aiModelsList();
    setList(next);
    return next;
  }, [api]);

  useEffect(() => {
    void api.aiServices().then(setServices);
    void api.aiProtocols().then(setProtocols);
    void reload().then((next) => {
      const first = next.entries.find((e) => e.id === next.active) ?? next.entries[0];
      if (first) {
        setSelection(first.id);
        open(first);
      } else {
        setSelection('new');
        open(null);
      }
    });
    const off = api.onAiModelsChanged(() => void reload());
    return () => void off.then((f) => f());
  }, [api, reload, open]);

  useEffect(() => {
    if (selection === null) return;
    const timer = setTimeout(() => {
      void api.aiHasKey(draft.service, draft.base).then(setSavedKey).catch(() => setSavedKey(false));
    }, 250);
    return () => clearTimeout(timer);
  }, [api, selection, draft.service, draft.base, list]);

  const select = (id: Selection) => {
    setSelection(id);
    open(id === 'new' || id === null ? null : (entries.find((e) => e.id === id) ?? null));
  };

  const run = async (name: string, task: () => Promise<void>) => {
    setBusy(name);
    setMessage(null);
    try {
      await task();
    } catch (error) {
      setMessage({ text: String(error), error: true });
    } finally {
      setBusy('');
    }
  };

  const set = <K extends keyof AiModelEntry>(field: K, value: AiModelEntry[K]) => {
    setDraft((old) => ({ ...old, [field]: value }));
    setArmed('');
  };

  const pickService = (id: string) => {
    // サービスを替えたら、別の接続先のキーやURL・モデル名を持ち越さない。
    setDraft((old) => ({ ...old, service: id, base: '', model: '', resolved: undefined }));
    setKey('');
    setCandidates([]);
    setMessage(null);
  };

  const save = () =>
    run('save', async () => {
      const before = new Set(entries.map((e) => e.id));
      const next = await api.aiModelSave(draft, key);
      setList(next);
      const saved = typeof selection === 'string' && selection !== 'new' ? selection : next.entries.find((e) => !before.has(e.id))?.id;
      if (saved) {
        setSelection(saved);
        const view = next.entries.find((e) => e.id === saved);
        if (view) setDraft({ ...view });
      }
      setKey('');
      setMessage(
        next.warning
          ? { text: next.warning, error: true }
          : { text: next.active === saved ? '保存しました（このモデルを使っています）' : '保存しました', error: false },
      );
    });

  const check = () =>
    run('check', async () => {
      const result = await api.aiModelCheck(draft, key);
      setCandidates(result.models);
      setMessage({
        text: result.ok ? `${result.message}（モデル: ${result.model ?? '-'}）` : result.message,
        error: !result.ok,
      });
    });

  const fetchModels = () =>
    run('models', async () => {
      const found = await api.aiModelCandidates(draft, key);
      setCandidates(found);
      setMessage({ text: found.length ? `${found.length}件のモデルが見つかりました` : 'モデルの一覧は空でした。名前を直接入力してください', error: found.length === 0 });
    });

  const use = (id: string) =>
    run('use', async () => {
      const result = await api.aiModelUse(id);
      await reload();
      setMessage({ text: `${result.model} を使います。会話は新しくなりました`, error: false });
    });

  const remove = (id: string) =>
    run('delete', async () => {
      const next = await api.aiModelDelete(id);
      setList(next);
      const first = next.entries[0];
      setSelection(first ? first.id : 'new');
      open(first ?? null);
    });

  const forgetKey = () =>
    run('forget', async () => {
      await api.aiForget(draft.service, draft.base);
      await reload();
      setArmed('');
      setMessage({ text: '保存したキーを削除しました', error: false });
    });

  const needsKeyNow = service?.needsKey === true && key.trim() === '' && !savedKey;
  const urlMissing = draft.service === 'custom' && !draft.base.trim();
  // 保存は通信しないので、キーが無くても進められる（あとで入れる）。確認・一覧取得はキーが要る。
  const canSave = service !== undefined && !urlMissing;
  const canCall = canSave && !needsKeyNow;

  return (
    <div className={styles.page}>
      <div className={styles.head}>
        <Title2>AIモデルの管理</Title2>
        <Caption1 className={styles.note}>APIキーはこのPCの専用の暗号化ファイルにだけ保存します。</Caption1>
      </div>
      <div className={styles.body}>
        <div className={styles.list}>
          <Button appearance="primary" icon={<Add20Regular />} onClick={() => select('new')}>
            AIモデルを追加
          </Button>
          {entries.map((e) => (
            <button
              key={e.id}
              type="button"
              className={mergeClasses(styles.item, selection === e.id && styles.itemSelected)}
              onClick={() => select(e.id)}
            >
              <Text weight="semibold">{e.label}</Text>
              <div className={styles.itemRow}>
                {list?.active === e.id && (
                  <Badge appearance="filled" color="brand" icon={<Checkmark20Regular />}>
                    使用中
                  </Badge>
                )}
                {!e.hasKey && (
                  <Badge appearance="tint" color="warning">
                    キー未登録
                  </Badge>
                )}
              </div>
            </button>
          ))}
        </div>

        <div className={styles.panel}>
          {selection === null ? (
            <Spinner size="small" label="読み込み中" />
          ) : (
            <>
              <Subtitle2>{selection === 'new' ? '新しいAIモデル' : '登録内容'}</Subtitle2>
              <Field label="サービス" hint={service?.blurb}>
                <Select value={draft.service} disabled={busy !== ''} onChange={(_, d) => pickService(d.value)}>
                  {services.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                    </option>
                  ))}
                </Select>
              </Field>
              {isUrl && (
                <Field label="接続先URL">
                  <Input
                    value={draft.base}
                    placeholder={service?.defaultBase || 'https://example.com/v1'}
                    onChange={(_, d) => set('base', d.value)}
                  />
                </Field>
              )}
              {service?.needsKey && (
                <Field
                  label="APIキー"
                  hint={savedKey ? '保存済みのキーを使います。変えるときだけ貼り付けてください。' : undefined}
                >
                  <div className={styles.row}>
                    <Input
                      className={styles.grow}
                      type="password"
                      autoComplete="off"
                      value={key}
                      placeholder={savedKey ? '保存済み' : 'ここに貼り付け'}
                      onChange={(_, d) => setKey(d.value)}
                    />
                    {service.hasKeyPage && (
                      <Link onClick={() => void api.aiOpenKeyPage(draft.service).catch((e) => setMessage({ text: String(e), error: true }))}>
                        キーの取得ページを開く
                      </Link>
                    )}
                  </div>
                </Field>
              )}
              <Field label="モデル名" hint="空のままなら、使うときに自動で選びます。">
                <div className={styles.row}>
                  <Input
                    className={styles.grow}
                    value={draft.model}
                    placeholder="例: claude-sonnet-5-5 / gpt-5"
                    onChange={(_, d) => set('model', d.value)}
                  />
                  {candidates.length > 0 && (
                    <Select value="" onChange={(_, d) => d.value && set('model', d.value)} style={{ maxWidth: 190 }}>
                      <option value="">一覧から選ぶ</option>
                      {candidates.map((m) => (
                        <option key={m} value={m}>
                          {m}
                        </option>
                      ))}
                    </Select>
                  )}
                  <Button size="small" disabled={busy !== '' || !canCall} onClick={() => void fetchModels()}>
                    {busy === 'models' ? '取得中…' : '一覧を取得'}
                  </Button>
                </div>
              </Field>
              <Accordion collapsible>
                <AccordionItem value="adv">
                  <AccordionHeader size="small">詳細</AccordionHeader>
                  <AccordionPanel>
                    <Field label="表示名">
                      <Input value={draft.name} placeholder="例: 仕事用 / 安いモデル" onChange={(_, d) => set('name', d.value)} />
                    </Field>
                    {isUrl && (
                      <>
                        <Field label="通信の形式">
                          <Select value={draft.protocol} onChange={(_, d) => set('protocol', d.value)}>
                            {protocols.map(([id, label]) => (
                              <option key={id} value={id}>
                                {label}
                              </option>
                            ))}
                          </Select>
                        </Field>
                        <Checkbox
                          checked={draft.useTools}
                          onChange={(_, d) => set('useTools', d.checked === true)}
                          label="関数呼び出し（ツール）に対応している"
                        />
                      </>
                    )}
                  </AccordionPanel>
                </AccordionItem>
              </Accordion>

              <div className={styles.actions}>
                <Button appearance="primary" disabled={busy !== '' || !canSave} onClick={() => void save()}>
                  {busy === 'save' ? '保存中…' : '保存'}
                </Button>
                <Button disabled={busy !== '' || !canCall} onClick={() => void check()}>
                  {busy === 'check' ? '確認中…' : '接続を確認'}
                </Button>
                {current && list?.active !== current.id && (
                  <Button disabled={busy !== '' || dirty} onClick={() => void use(current.id)}>
                    このモデルを使う
                  </Button>
                )}
                {busy !== '' && <Spinner size="tiny" />}
                {dirty && selection !== 'new' && <Caption1 className={styles.note}>未保存の変更があります</Caption1>}
              </div>
              {message && (
                <MessageBar intent={message.error ? 'error' : 'success'}>
                  <MessageBarBody>{message.text}</MessageBarBody>
                </MessageBar>
              )}

              {current && (
                <Accordion collapsible>
                  <AccordionItem value="delete">
                    <AccordionHeader size="small">削除</AccordionHeader>
                    <AccordionPanel>
                      <div className={styles.actions}>
                        <Button
                          appearance="outline"
                          disabled={busy !== ''}
                          onClick={() => (armed === 'model' ? void remove(current.id) : setArmed('model'))}
                        >
                          {armed === 'model' ? 'もう一度押すと削除' : 'この登録を削除'}
                        </Button>
                        <Caption1 className={styles.note}>キーは残ります。</Caption1>
                      </div>
                      {service?.needsKey && savedKey && (
                        <div className={styles.actions} style={{ marginTop: 8 }}>
                          <Button
                            appearance="outline"
                            disabled={busy !== ''}
                            onClick={() => (armed === 'key' ? void forgetKey() : setArmed('key'))}
                          >
                            {armed === 'key' ? 'もう一度押すと削除' : '保存したキーを削除'}
                          </Button>
                          <Caption1 className={styles.note}>同じサービス・URLの全登録で使えなくなります。</Caption1>
                        </div>
                      )}
                    </AccordionPanel>
                  </AccordionItem>
                </Accordion>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
