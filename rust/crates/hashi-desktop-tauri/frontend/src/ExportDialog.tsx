// 接続先の書き出し（hashi-export 形式。旧版の「接続情報の書き出し」と同じ）。
// 書き出したファイルは「取り込む」で読める。パスワード等は、決めたパスフレーズで暗号化したときだけ含める。
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
  MessageBar,
  MessageBarBody,
  Text,
  tokens,
} from '@fluentui/react-components';
import type { Api } from './api';
import { t } from './i18n';
import type { Profile } from './types';

const MIN_PASSPHRASE = 8;

const useStyles = makeStyles({
  body: { display: 'flex', flexDirection: 'column', gap: '12px' },
  list: {
    display: 'flex',
    flexDirection: 'column',
    gap: '2px',
    maxHeight: '220px',
    overflowY: 'auto',
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    padding: '4px 8px',
  },
  item: { display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 },
  itemText: { display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1 },
  head: { display: 'flex', alignItems: 'center', gap: '8px' },
  grow: { flex: 1 },
  actions: { whiteSpace: 'nowrap', minWidth: 'auto' },
  pass: { display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px', '> *': { minWidth: 0 } },
});

interface Props {
  api: Api;
  open: boolean;
  profiles: Profile[];
  /** 最初から選んでおく接続先（接続の編集画面から開いたときはその1件）。 */
  initial: number[];
  onClose(): void;
  onDone(summary: string): void;
}

export function ExportDialog({ api, open, profiles, initial, onClose, onDone }: Props) {
  const styles = useStyles();
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [withSecrets, setWithSecrets] = useState(false);
  const [pass, setPass] = useState('');
  const [again, setAgain] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!open) return;
    setSelected(new Set(initial));
    setWithSecrets(false);
    setPass('');
    setAgain('');
    setError('');
  }, [open, initial]);

  const passProblem = !withSecrets
    ? null
    : pass.length < MIN_PASSPHRASE
      ? t('パスフレーズは{n}文字以上にしてください', { n: MIN_PASSPHRASE })
      : pass !== again
        ? t('確認用のパスフレーズが一致しません')
        : null;
  const ready = selected.size > 0 && passProblem === null && !busy;

  const run = async () => {
    setBusy(true);
    setError('');
    try {
      const indices = [...selected].sort((a, b) => a - b);
      const r = await api.exportBundle(indices, withSecrets ? pass : null);
      if (!r) return; // 保存先の選択を取り消した
      setPass('');
      setAgain('');
      onDone(
        withSecrets
          ? t('{file} へ{n}件の接続先を書き出しました（ホスト鍵{h}件・暗号化した秘密{s}件）', { file: r.file_name, n: r.profiles, h: r.hosts, s: r.secrets })
          : t('{file} へ{n}件の接続先を書き出しました（ホスト鍵{h}件・パスワードは含めていません）', { file: r.file_name, n: r.profiles, h: r.hosts }),
      );
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const all = selected.size === profiles.length && profiles.length > 0;
  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 'min(600px, calc(100vw - 32px))' }}>
        <DialogBody>
          <DialogTitle>{t('接続先を書き出す')}</DialogTitle>
          <DialogContent className={styles.body}>
            <Body1>
              {t('選んだ接続先の設定と承認済みのホスト鍵を1つのファイルにします。別のパソコンの Hashi（旧版を含む）の「取り込む」で読み込めます。')}
            </Body1>
            <div className={styles.head}>
              <Text weight="semibold" className={styles.grow}>
                {t('接続先（{n}件を選択）', { n: selected.size })}
              </Text>
              <Button
                size="small"
                appearance="subtle"
                onClick={() => setSelected(all ? new Set() : new Set(profiles.map((_, i) => i)))}
              >
                {all ? t('すべて外す') : t('すべて選ぶ')}
              </Button>
            </div>
            <div className={styles.list}>
              {profiles.map((p, index) => (
                <div key={index} className={styles.item}>
                  <Checkbox
                    checked={selected.has(index)}
                    aria-label={t('{label}を書き出す', { label: p.name || p.host })}
                    onChange={(_, d) =>
                      setSelected((old) => {
                        const next = new Set(old);
                        if (d.checked) next.add(index);
                        else next.delete(index);
                        return next;
                      })
                    }
                  />
                  <div className={styles.itemText}>
                    <Text truncate wrap={false}>
                      {p.name || p.host}
                    </Text>
                    <Caption1 truncate wrap={false}>
                      {p.username}@{p.host}:{p.port}
                    </Caption1>
                  </div>
                </div>
              ))}
            </div>
            <Checkbox
              checked={withSecrets}
              onChange={(_, d) => setWithSecrets(Boolean(d.checked))}
              label={t('保存済みのパスワード・パスフレーズも含める（パスフレーズで暗号化）')}
            />
            {withSecrets && (
              <>
                <div className={styles.pass}>
                  <Field label={t('パスフレーズ')}>
                    <Input type="password" autoComplete="off" value={pass} onChange={(_, d) => setPass(d.value)} />
                  </Field>
                  <Field label={t('パスフレーズ（確認）')}>
                    <Input type="password" autoComplete="off" value={again} onChange={(_, d) => setAgain(d.value)} />
                  </Field>
                </div>
                <Caption1>
                  {passProblem ?? t('取り込むときにこのパスフレーズが必要です。忘れると、パスワードは取り出せません（接続先の設定は読めます）。')}
                </Caption1>
              </>
            )}
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            <Button className={styles.actions} onClick={onClose}>
              {t('キャンセル')}
            </Button>
            <Button className={styles.actions} appearance="primary" disabled={!ready} onClick={() => void run()}>
              {t('書き出す…')}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
