import { useEffect, useState } from 'react';
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
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
  MessageBar,
  MessageBarBody,
  Tab,
  TabList,
  tokens,
} from '@fluentui/react-components';
import type { Profile } from './types';

const useStyles = makeStyles({
  form: { display: 'flex', flexDirection: 'column', gap: '12px' },
  row: { display: 'grid', gridTemplateColumns: 'minmax(0, 1fr) 120px', gap: '12px', '> *': { minWidth: 0 } },
  actions: { whiteSpace: 'nowrap', minWidth: 'auto' },
  keyRow: { display: 'flex', gap: '8px' },
  grow: { flex: 1 },
});

interface Props {
  open: boolean;
  initial: Profile;
  existing: boolean;
  error: string | null;
  onSave(profile: Profile, connect: boolean): void;
  onDelete(): void;
  onClose(): void;
  onPickKey(): Promise<string | null>;
}

export function ProfileDialog({ open, initial, existing, error, onSave, onDelete, onClose, onPickKey }: Props) {
  const styles = useStyles();
  const [p, setP] = useState<Profile>(initial);
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    setP(initial);
    setArmed(false);
  }, [initial, open]);
  const set = <K extends keyof Profile>(key: K, value: Profile[K]) => setP((old) => ({ ...old, [key]: value }));
  const ready = p.host.trim() !== '' && p.username.trim() !== '';
  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
        <DialogBody>
          <DialogTitle>{existing ? '接続を編集' : '新しい接続'}</DialogTitle>
          <DialogContent className={styles.form}>
            <Field label="名前">
              <Input
                value={p.name}
                placeholder="例: 本番サーバー（空ならホスト名）"
                onChange={(_, d) => set('name', d.value)}
              />
            </Field>
            <div className={styles.row}>
              <Field label="ホスト" required>
                <Input value={p.host} placeholder="example.com" onChange={(_, d) => set('host', d.value)} />
              </Field>
              <Field label="ポート">
                <Input
                  type="number"
                  style={{ width: '100%', minWidth: 0 }}
                  value={String(p.port)}
                  onChange={(_, d) => set('port', Math.min(65535, Math.max(1, Number(d.value) || 22)))}
                />
              </Field>
            </div>
            <Field label="利用者名" required>
              <Input value={p.username} onChange={(_, d) => set('username', d.value)} />
            </Field>
            <Field label="ログイン方法">
              <TabList
                selectedValue={p.auth_method}
                onTabSelect={(_, d) => set('auth_method', d.value as Profile['auth_method'])}
              >
                <Tab value="key">秘密鍵</Tab>
                <Tab value="password">パスワード</Tab>
                <Tab value="agent">SSH agent</Tab>
              </TabList>
            </Field>
            {p.auth_method === 'key' && (
              <div className={styles.keyRow}>
                <Input
                  className={styles.grow}
                  value={p.key_path}
                  placeholder="空なら ~/.ssh の標準の鍵を探します"
                  onChange={(_, d) => set('key_path', d.value)}
                />
                <Button
                  onClick={async () => {
                    const path = await onPickKey();
                    if (path) set('key_path', path);
                  }}
                >
                  選ぶ
                </Button>
              </div>
            )}
            {p.auth_method === 'password' && <Caption1>パスワードは接続するときに入力します。</Caption1>}
            {p.auth_method === 'agent' && <Caption1>起動中のSSH agentを使います。</Caption1>}
            <Accordion collapsible>
              <AccordionItem value="more">
                <AccordionHeader>詳細設定</AccordionHeader>
                <AccordionPanel className={styles.form}>
                  <Field label="踏み台（ProxyJump）">
                    <Input
                      value={p.proxy_jump}
                      placeholder="例: user@bastion.example.com"
                      onChange={(_, d) => set('proxy_jump', d.value)}
                    />
                  </Field>
                  <Field label="ファイル画面の開始フォルダ">
                    <Input
                      value={p.initial_path}
                      placeholder="空ならホームフォルダ"
                      onChange={(_, d) => set('initial_path', d.value)}
                    />
                  </Field>
                  <Checkbox
                    checked={p.agent_forwarding}
                    onChange={(_, d) => set('agent_forwarding', d.checked === true)}
                    label="SSH agentを接続先へ転送する（接続先に署名を許可します）"
                  />
                </AccordionPanel>
              </AccordionItem>
            </Accordion>
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            {existing && (
              <Button
                appearance="subtle"
                style={{ marginRight: 'auto', color: tokens.colorPaletteRedForeground1 }}
                onClick={() => (armed ? onDelete() : setArmed(true))}
              >
                {armed ? 'もう一度押すと削除' : '削除'}
              </Button>
            )}
            <Button className={styles.actions} onClick={onClose}>
              キャンセル
            </Button>
            <Button className={styles.actions} disabled={!ready} onClick={() => onSave(p, false)}>
              保存
            </Button>
            <Button className={styles.actions} appearance="primary" disabled={!ready} onClick={() => onSave(p, true)}>
              保存して接続
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
