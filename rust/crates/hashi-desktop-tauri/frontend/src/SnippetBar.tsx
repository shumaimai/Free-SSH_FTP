import { useEffect, useState } from 'react';
import {
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
  MenuDivider,
  MenuItem,
  MenuList,
  MenuPopover,
  MenuTrigger,
  MessageBar,
  MessageBarBody,
  Textarea,
  tokens,
} from '@fluentui/react-components';
import { Add20Regular, Code20Regular, Delete20Regular, Edit20Regular } from '@fluentui/react-icons';
import type { Api } from './api';
import type { Snippet } from './types';

const useStyles = makeStyles({
  bar: {
    display: 'flex',
    alignItems: 'center',
    gap: '4px',
    padding: '2px 8px',
    minHeight: '34px',
    backgroundColor: tokens.colorNeutralBackground3,
    borderBottom: `1px solid ${tokens.colorNeutralStroke2}`,
  },
  grow: { flex: 1 },
  manager: { display: 'grid', gridTemplateColumns: '200px 1fr', gap: '14px', minHeight: '300px' },
  list: { display: 'flex', flexDirection: 'column', gap: '2px', overflowY: 'auto', maxHeight: '340px' },
  form: { display: 'flex', flexDirection: 'column', gap: '10px' },
  mono: { fontFamily: 'Consolas, "Cascadia Mono", monospace' },
  note: { color: tokens.colorNeutralForeground3 },
});

interface Props {
  api: Api;
  /** 送り先の端末。接続できていなければnull。 */
  sessionId: string | null;
  onError(message: string): void;
}

/** 端末の上の細い操作列。スニペットの送信と管理。 */
export function SnippetBar({ api, sessionId, onError }: Props) {
  const styles = useStyles();
  const [list, setList] = useState<Snippet[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [managing, setManaging] = useState(false);
  const [pending, setPending] = useState<{ index: number; variables: string[] } | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});

  const reload = () =>
    api
      .snippetsList()
      .then((l) => {
        setList(l);
        setLoadError(null);
      })
      .catch((e) => setLoadError(String(e)));
  useEffect(() => void reload(), []); // eslint-disable-line react-hooks/exhaustive-deps

  const send = async (index: number) => {
    if (!sessionId) return;
    try {
      const variables = await api.snippetVariables(list[index].body);
      if (variables.length === 0) await api.snippetSend(sessionId, index, {});
      else {
        setValues({});
        setPending({ index, variables });
      }
    } catch (e) {
      onError(String(e));
    }
  };

  return (
    <div className={styles.bar}>
      <Menu onOpenChange={(_, d) => d.open && void reload()}>
        <MenuTrigger disableButtonEnhancement>
          <Button size="small" appearance="subtle" icon={<Code20Regular />} disabled={!sessionId}>
            スニペット
          </Button>
        </MenuTrigger>
        <MenuPopover>
          <MenuList>
            {loadError && <MenuItem disabled>{loadError}</MenuItem>}
            {list.map((snippet, index) => (
              <MenuItem key={index} onClick={() => void send(index)}>
                {snippet.name}
              </MenuItem>
            ))}
            {list.length === 0 && !loadError && <MenuItem disabled>スニペットはまだありません</MenuItem>}
            <MenuDivider />
            <MenuItem icon={<Edit20Regular />} onClick={() => setManaging(true)}>
              管理...
            </MenuItem>
          </MenuList>
        </MenuPopover>
      </Menu>
      <div className={styles.grow} />
      <Dialog open={pending !== null} onOpenChange={(_, d) => !d.open && setPending(null)}>
        <DialogSurface aria-describedby={undefined}>
          <DialogBody>
            <DialogTitle>{pending ? list[pending.index]?.name : ''}</DialogTitle>
            <DialogContent className={styles.form}>
              {pending?.variables.map((name) => (
                <Field key={name} label={name}>
                  <Input
                    value={values[name] ?? ''}
                    onChange={(_, d) => setValues((old) => ({ ...old, [name]: d.value }))}
                  />
                </Field>
              ))}
              <Caption1 className={styles.note}>入力しなかった変数は、{'{{名前}}'}のまま送られます。</Caption1>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setPending(null)}>やめる</Button>
              <Button
                appearance="primary"
                onClick={() => {
                  const current = pending;
                  setPending(null);
                  if (current && sessionId)
                    void api.snippetSend(sessionId, current.index, values).catch((e) => onError(String(e)));
                }}
              >
                送る
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <SnippetManager
        api={api}
        open={managing}
        list={list}
        loadError={loadError}
        onClose={() => setManaging(false)}
        onSaved={setList}
      />
    </div>
  );
}

function SnippetManager({
  api,
  open,
  list,
  loadError,
  onClose,
  onSaved,
}: {
  api: Api;
  open: boolean;
  list: Snippet[];
  loadError: string | null;
  onClose(): void;
  onSaved(list: Snippet[]): void;
}) {
  const styles = useStyles();
  const [draft, setDraft] = useState<Snippet[]>([]);
  const [current, setCurrent] = useState(0);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (open) {
      setDraft(list.map((s) => ({ ...s })));
      setCurrent(0);
      setError(null);
    }
  }, [open, list]);
  const item = draft[current];
  const update = (patch: Partial<Snippet>) =>
    setDraft((old) => old.map((s, i) => (i === current ? { ...s, ...patch } : s)));
  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 720 }}>
        <DialogBody>
          <DialogTitle>スニペットの管理</DialogTitle>
          <DialogContent>
            {loadError && (
              <MessageBar intent="error">
                <MessageBarBody>{loadError}（保存済みの記録を守るため、編集は無効です）</MessageBarBody>
              </MessageBar>
            )}
            <div className={styles.manager}>
              <div className={styles.list}>
                {draft.map((s, i) => (
                  <Button
                    key={i}
                    appearance={i === current ? 'primary' : 'subtle'}
                    style={{ justifyContent: 'flex-start' }}
                    onClick={() => setCurrent(i)}
                  >
                    {s.name || '（名前なし）'}
                  </Button>
                ))}
                <Button
                  icon={<Add20Regular />}
                  disabled={loadError !== null}
                  onClick={() => {
                    setDraft((old) => [...old, { name: '新しいスニペット', body: '', send_enter: true }]);
                    setCurrent(draft.length);
                  }}
                >
                  追加
                </Button>
              </div>
              {item ? (
                <div className={styles.form}>
                  <Field label="名前">
                    <Input value={item.name} onChange={(_, d) => update({ name: d.value })} />
                  </Field>
                  <Field label="本文（{{名前}} は送るときに入力できます）">
                    <Textarea
                      textarea={{ className: styles.mono }}
                      rows={6}
                      value={item.body}
                      onChange={(_, d) => update({ body: d.value })}
                    />
                  </Field>
                  <Checkbox
                    checked={item.send_enter}
                    onChange={(_, d) => update({ send_enter: d.checked === true })}
                    label="送ったあとにEnterを押す"
                  />
                  <Button
                    icon={<Delete20Regular />}
                    appearance="subtle"
                    style={{ alignSelf: 'flex-start', color: tokens.colorPaletteRedForeground1 }}
                    onClick={() => {
                      setDraft((old) => old.filter((_, i) => i !== current));
                      setCurrent(0);
                    }}
                  >
                    このスニペットを削除
                  </Button>
                </div>
              ) : (
                <Body1 className={styles.note}>左の「追加」から作ります。</Body1>
              )}
            </div>
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            <Button onClick={onClose}>キャンセル</Button>
            <Button
              appearance="primary"
              disabled={loadError !== null}
              onClick={() =>
                void api
                  .snippetsSave(draft)
                  .then((saved) => {
                    onSaved(saved);
                    onClose();
                  })
                  .catch((e) => setError(String(e)))
              }
            >
              保存
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
