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
import { t } from './i18n';
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
  /** この接続先に保存されている秘密の種類（値は持たない）。 */
  savedSecrets: string[];
  onForgetSecrets(): Promise<void>;
}

export function ProfileDialog({ open, initial, existing, error, onSave, onDelete, onClose, onPickKey, savedSecrets, onForgetSecrets }: Props) {
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
          <DialogTitle>{existing ? t('接続を編集') : t('新しい接続')}</DialogTitle>
          <DialogContent className={styles.form}>
            <Field label={t('名前')}>
              <Input
                value={p.name}
                placeholder={t('例: 本番サーバー（空ならホスト名）')}
                onChange={(_, d) => set('name', d.value)}
              />
            </Field>
            <div className={styles.row}>
              <Field label={t('ホスト')} required>
                <Input value={p.host} placeholder="example.com" onChange={(_, d) => set('host', d.value)} />
              </Field>
              <Field label={t('ポート')}>
                <Input
                  type="number"
                  style={{ width: '100%', minWidth: 0 }}
                  value={String(p.port)}
                  onChange={(_, d) => set('port', Math.min(65535, Math.max(1, Number(d.value) || 22)))}
                />
              </Field>
            </div>
            <Field label={t('利用者名')} required>
              <Input value={p.username} onChange={(_, d) => set('username', d.value)} />
            </Field>
            <Field label={t('ログイン方法')}>
              <TabList
                selectedValue={p.auth_method}
                onTabSelect={(_, d) => set('auth_method', d.value as Profile['auth_method'])}
              >
                <Tab value="key">{t('秘密鍵')}</Tab>
                <Tab value="password">{t('パスワード')}</Tab>
                <Tab value="agent">SSH agent</Tab>
              </TabList>
            </Field>
            {p.auth_method === 'key' && (
              <div className={styles.keyRow}>
                <Input
                  className={styles.grow}
                  value={p.key_path}
                  placeholder={t('空なら ~/.ssh の標準の鍵を探します')}
                  onChange={(_, d) => set('key_path', d.value)}
                />
                <Button
                  onClick={async () => {
                    const path = await onPickKey();
                    if (path) set('key_path', path);
                  }}
                >
                  {t('選ぶ')}
                </Button>
              </div>
            )}
            {p.auth_method === 'password' && (
              <Caption1>
                {savedSecrets.includes('password') ? t('保存済みのパスワードを使います。拒否されたときは入力を求めます。') : t('パスワードは接続するときに入力します。')}
              </Caption1>
            )}
            {existing && savedSecrets.length > 0 && (
              <div className={styles.keyRow}>
                <Caption1 className={styles.grow}>
                  {t('保存済み: {names}', {
                    names: savedSecrets.map((k) => (k === 'password' ? t('パスワード') : t('パスフレーズ'))).join(t('・')),
                  })}
                </Caption1>
                <Button size="small" appearance="subtle" onClick={() => void onForgetSecrets()}>
                  {t('保存済みの秘密を削除')}
                </Button>
              </div>
            )}
            {p.auth_method === 'agent' && <Caption1>{t('起動中のSSH agentを使います。')}</Caption1>}
            <Accordion collapsible>
              <AccordionItem value="more">
                <AccordionHeader>{t('詳細設定')}</AccordionHeader>
                <AccordionPanel className={styles.form}>
                  <Field label={t('踏み台（ProxyJump）')}>
                    <Input
                      value={p.proxy_jump}
                      placeholder={t('例: user@bastion.example.com')}
                      onChange={(_, d) => set('proxy_jump', d.value)}
                    />
                  </Field>
                  <Field label={t('ファイル画面の開始フォルダ')}>
                    <Input
                      value={p.initial_path}
                      placeholder={t('空ならホームフォルダ')}
                      onChange={(_, d) => set('initial_path', d.value)}
                    />
                  </Field>
                  <Checkbox
                    checked={p.agent_forwarding}
                    onChange={(_, d) => set('agent_forwarding', d.checked === true)}
                    label={t('SSH agentを接続先へ転送する（接続先に署名を許可します）')}
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
                {armed ? t('もう一度押すと削除') : t('削除')}
              </Button>
            )}
            <Button className={styles.actions} onClick={onClose}>
              {t('キャンセル')}
            </Button>
            <Button className={styles.actions} disabled={!ready} onClick={() => onSave(p, false)}>
              {t('保存')}
            </Button>
            <Button className={styles.actions} appearance="primary" disabled={!ready} onClick={() => onSave(p, true)}>
              {t('保存して接続')}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
