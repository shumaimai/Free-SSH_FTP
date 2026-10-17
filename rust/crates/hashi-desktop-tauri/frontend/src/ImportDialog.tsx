// 旧Python版からの取り込み。旧版の保存先を直接読む方法と、旧版が書き出したファイルを読む方法がある。
// パスワード等の値は画面へ来ない（有無だけ）。取り込みは確認のあと、選んだ項目だけを保存する。
import { useEffect, useState } from 'react';
import {
  Badge,
  Body1,
  Button,
  Caption1,
  Card,
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
  Spinner,
  Subtitle2,
  Text,
  tokens,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { ImportPreview, ImportResult, Profile } from './types';

const useStyles = makeStyles({
  body: { display: 'flex', flexDirection: 'column', gap: '12px' },
  list: {
    display: 'flex',
    flexDirection: 'column',
    gap: '2px',
    maxHeight: '240px',
    overflowY: 'auto',
    border: `1px solid ${tokens.colorNeutralStroke2}`,
    borderRadius: tokens.borderRadiusMedium,
    padding: '4px 8px',
  },
  item: { display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 },
  itemText: { display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1 },
  badges: { display: 'flex', gap: '4px', flexWrap: 'wrap' },
  choose: { display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '12px' },
  card: { display: 'flex', flexDirection: 'column', gap: '8px', padding: '12px' },
  passRow: { display: 'flex', gap: '8px', alignItems: 'flex-end' },
});

interface Props {
  api: Api;
  open: boolean;
  onClose(): void;
  onDone(profiles: Profile[], summary: string): void;
}

export function ImportDialog({ api, open, onClose, onDone }: Props) {
  const styles = useStyles();
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [overwrite, setOverwrite] = useState(false);
  const [knownHosts, setKnownHosts] = useState(true);
  const [snippets, setSnippets] = useState(true);
  const [secrets, setSecrets] = useState(true);
  const [legacyStopped, setLegacyStopped] = useState(false);
  const [passphrase, setPassphrase] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [result, setResult] = useState<ImportResult | null>(null);

  useEffect(() => {
    if (!open) return;
    setPreview(null);
    setResult(null);
    setError('');
    setLegacyStopped(false);
    setPassphrase('');
    setOverwrite(false);
  }, [open]);

  const adopt = (p: ImportPreview) => {
    setPreview(p);
    setSelected(new Set(p.profiles.filter((x) => !x.exists).map((x) => x.index)));
    setSecrets(true);
  };

  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    setError('');
    try {
      await task();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const close = () => {
    void api.importCancel();
    onClose();
  };

  const secretCount = preview ? preview.profiles.filter((p) => p.password || p.passphrase).length : 0;
  const canApply =
    preview !== null && !busy && [...selected].some((i) => overwrite || !preview.profiles[i]?.exists);

  const apply = () =>
    run(async () => {
      if (!preview) return;
      const [r, profiles] = await api.importApply({
        token: preview.token,
        selected: [...selected].filter((i) => overwrite || !preview.profiles[i]?.exists),
        overwrite,
        known_hosts: knownHosts,
        snippets: snippets && preview.snippets_new !== null && preview.snippets_new > 0,
        secrets,
      });
      setResult(r);
      setPreview(null);
      onDone(
        profiles,
        `${r.added + r.updated}件の接続先を取り込みました${r.secrets_saved ? `（保存済みの秘密${r.secrets_saved}件）` : ''}`,
      );
    });

  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && close()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 'min(680px, calc(100vw - 32px))' }}>
        <DialogBody>
          <DialogTitle>旧版（Python）から取り込む</DialogTitle>
          <DialogContent className={styles.body}>
            {!preview && !result && (
              <>
                <Body1>旧版の接続先・既知のホスト鍵・スニペット・保存済みのパスワードを、このアプリへコピーします。旧版のデータは読むだけで、変更しません。</Body1>
                <div className={styles.choose}>
                  <Card className={styles.card}>
                    <Subtitle2>旧版のデータを直接読む</Subtitle2>
                    <Caption1>
                      旧版の設定フォルダと、旧版が使っていた保存先（Windows資格情報マネージャ／暗号化ファイル）から読みます。
                    </Caption1>
                    <Checkbox
                      checked={legacyStopped}
                      onChange={(_, d) => setLegacyStopped(Boolean(d.checked))}
                      label="旧Python版をすべて終了した"
                    />
                    <Button
                      appearance="primary"
                      disabled={busy || !legacyStopped}
                      onClick={() => void run(async () => adopt(await api.importLegacyInspect(legacyStopped)))}
                    >
                      読み込む
                    </Button>
                  </Card>
                  <Card className={styles.card}>
                    <Subtitle2>書き出しファイルを選ぶ</Subtitle2>
                    <Caption1>旧版の「接続情報の書き出し」で作ったファイル（hashi-export）を読みます。パスワードは書き出し時のパスフレーズで復号します。</Caption1>
                    <Button
                      disabled={busy}
                      onClick={() =>
                        void run(async () => {
                          const p = await api.importBundlePick();
                          if (p) adopt(p);
                        })
                      }
                    >
                      ファイルを選ぶ…
                    </Button>
                  </Card>
                </div>
                {busy && <Spinner size="tiny" label="読み込み中" />}
              </>
            )}

            {preview && (
              <>
                <Text weight="semibold">接続先（{preview.profiles.length}件）</Text>
                <div className={styles.list}>
                  {preview.profiles.length === 0 && <Caption1>取り込める接続先がありません</Caption1>}
                  {preview.profiles.map((p) => (
                    <div key={p.index} className={styles.item}>
                      <Checkbox
                        checked={selected.has(p.index) && (overwrite || !p.exists)}
                        disabled={p.exists && !overwrite}
                        aria-label={`${p.label}を取り込む`}
                        onChange={(_, d) =>
                          setSelected((old) => {
                            const next = new Set(old);
                            if (d.checked) next.add(p.index);
                            else next.delete(p.index);
                            return next;
                          })
                        }
                      />
                      <div className={styles.itemText}>
                        <Text truncate wrap={false}>
                          {p.label}
                        </Text>
                        <Caption1 truncate wrap={false}>
                          {p.id}
                        </Caption1>
                      </div>
                      <div className={styles.badges}>
                        {p.exists && (
                          <Badge appearance="tint" color="warning">
                            登録済み
                          </Badge>
                        )}
                        {p.proxy_jump && (
                          <Badge appearance="tint" color="informative">
                            踏み台
                          </Badge>
                        )}
                        {p.password && (
                          <Badge appearance="tint" color="success">
                            パスワードあり
                          </Badge>
                        )}
                        {p.passphrase && (
                          <Badge appearance="tint" color="success">
                            パスフレーズあり
                          </Badge>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
                {preview.profiles.some((p) => p.exists) && (
                  <Checkbox
                    checked={overwrite}
                    onChange={(_, d) => setOverwrite(Boolean(d.checked))}
                    label="登録済みの接続先も、旧版の内容で置き換える"
                  />
                )}

                {preview.needs_passphrase && !preview.unlocked && (
                  <div className={styles.passRow}>
                    <Field label="書き出し時のパスフレーズ（保存済みのパスワードを読むため）" style={{ flex: 1 }}>
                      <Input
                        type="password"
                        value={passphrase}
                        onChange={(_, d) => setPassphrase(d.value)}
                        autoComplete="off"
                      />
                    </Field>
                    <Button
                      disabled={busy || passphrase === ''}
                      onClick={() =>
                        void run(async () => {
                          const p = await api.importBundleUnlock(preview.token, passphrase);
                          setPassphrase('');
                          setPreview(p);
                        })
                      }
                    >
                      復号する
                    </Button>
                  </div>
                )}

                <Text weight="semibold">あわせて取り込む</Text>
                <Checkbox
                  checked={knownHosts}
                  onChange={(_, d) => setKnownHosts(Boolean(d.checked))}
                  label={`承認済みのホスト鍵（新規${preview.hosts_new}件）${
                    preview.hosts_conflict > 0 ? `。鍵が食い違う${preview.hosts_conflict}件は上書きしません` : ''
                  }`}
                />
                {preview.snippets_new !== null && (
                  <Checkbox
                    checked={snippets && preview.snippets_new > 0}
                    disabled={preview.snippets_new === 0}
                    onChange={(_, d) => setSnippets(Boolean(d.checked))}
                    label={`スニペット（新規${preview.snippets_new}件）`}
                  />
                )}
                <Checkbox
                  checked={secrets && secretCount > 0}
                  disabled={secretCount === 0}
                  onChange={(_, d) => setSecrets(Boolean(d.checked))}
                  label={`保存済みのパスワード・パスフレーズ（${secretCount}件の接続先）をこのアプリに保存する`}
                />
                {secretCount === 0 && !preview.needs_passphrase && (
                  <Caption1>取り込める保存済みの秘密は見つかりませんでした。接続するときに入力してください。</Caption1>
                )}
                {preview.notes.map((n) => (
                  <Caption1 key={n}>{n}</Caption1>
                ))}
              </>
            )}

            {result && (
              <MessageBar intent={result.secrets_failed > 0 ? 'warning' : 'success'}>
                <MessageBarBody>
                  接続先: 追加{result.added}件・置き換え{result.updated}件・見送り{result.skipped}件。ホスト鍵: {result.hosts_added}件追加
                  {result.host_conflicts.length > 0 && `（食い違い${result.host_conflicts.length}件は変更なし: ${result.host_conflicts.join(', ')}）`}
                  。スニペット: {result.snippets_added}件追加。保存した秘密: {result.secrets_saved}件
                  {result.secrets_failed > 0 && `（${result.secrets_failed}件は保存できませんでした）`}。
                  {result.notes.map((n) => ` ${n}。`)}
                </MessageBarBody>
              </MessageBar>
            )}

            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            {preview && (
              <Button appearance="primary" style={{ whiteSpace: 'nowrap' }} disabled={!canApply} onClick={() => void apply()}>
                選んだ項目を取り込む
              </Button>
            )}
            {preview && (
              <Button
                onClick={() => {
                  void api.importCancel();
                  setPreview(null);
                }}
              >
                戻る
              </Button>
            )}
            <Button onClick={close}>{result ? '閉じる' : 'キャンセル'}</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
