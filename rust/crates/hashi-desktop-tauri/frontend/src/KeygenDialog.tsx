import { useEffect, useState } from 'react';
import {
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
  Radio,
  RadioGroup,
  Select,
  Spinner,
  Textarea,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { KeygenDefaults, KeygenResult } from './types';

const useStyles = makeStyles({
  form: { display: 'flex', flexDirection: 'column', gap: '12px' },
  row: { display: 'flex', gap: '8px' },
  grow: { flex: 1 },
  mono: { fontFamily: 'Consolas, "Cascadia Mono", monospace', fontSize: '12px' },
});

type Kind = 'ed25519' | 'ecdsa' | 'rsa';

interface Props {
  api: Api;
  open: boolean;
  /** 公開鍵を登録できる接続（開いているSSH接続）。 */
  registerSession: string | null;
  onClose(): void;
}

export function KeygenDialog({ api, open, registerSession, onClose }: Props) {
  const styles = useStyles();
  const [defaults, setDefaults] = useState<KeygenDefaults | null>(null);
  const [kind, setKind] = useState<Kind>('ed25519');
  const [bits, setBits] = useState(0);
  const [passphrase, setPassphrase] = useState('');
  const [confirm, setConfirm] = useState('');
  const [comment, setComment] = useState('');
  const [path, setPath] = useState('');
  const [exists, setExists] = useState(false);
  const [overwrite, setOverwrite] = useState(false);
  const [register, setRegister] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<KeygenResult | null>(null);

  const defaultPath = (d: KeygenDefaults, k: Kind) => (k === 'ed25519' ? d.ed25519 : k === 'ecdsa' ? d.ecdsa : d.rsa);
  const choices = defaults ? (kind === 'ecdsa' ? defaults.ecdsaBits : kind === 'rsa' ? defaults.rsaBits : []) : [];

  useEffect(() => {
    if (!open) return;
    setPassphrase('');
    setConfirm('');
    setResult(null);
    setError(null);
    setOverwrite(false);
    setRegister(false);
    void api.keygenDefaults().then((d) => {
      setDefaults(d);
      setPath(defaultPath(d, 'ed25519'));
      setKind('ed25519');
      setBits(0);
    });
  }, [open, api]);

  useEffect(() => {
    if (!path.trim()) return setExists(false);
    const timer = window.setTimeout(() => void api.keygenPathExists(path).then(setExists), 200);
    return () => window.clearTimeout(timer);
  }, [path, api]);

  const changeKind = (next: Kind) => {
    if (defaults && path === defaultPath(defaults, kind)) setPath(defaultPath(defaults, next));
    setKind(next);
    setBits(0);
  };

  const mismatch = passphrase !== confirm;
  const ready = !busy && !mismatch && path.trim() !== '' && (!exists || overwrite);

  const generate = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    const args = {
      kind,
      bits: choices.length > 0 ? choices[bits] : 0,
      passphrase,
      comment,
      path,
      overwrite,
      registerSession: register ? registerSession : null,
    };
    setPassphrase('');
    setConfirm('');
    try {
      setResult(await api.keygenGenerate(args));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && !busy && onClose()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
        <DialogBody>
          <DialogTitle>SSH鍵を作る</DialogTitle>
          <DialogContent className={styles.form}>
            <Field label="鍵の種類">
              <RadioGroup layout="horizontal" value={kind} onChange={(_, d) => changeKind(d.value as Kind)} disabled={busy}>
                <Radio value="ed25519" label="Ed25519（おすすめ）" />
                <Radio value="ecdsa" label="ECDSA" />
                <Radio value="rsa" label="RSA" />
              </RadioGroup>
            </Field>
            {choices.length > 0 && (
              <Field label="ビット数">
                <Select value={String(bits)} onChange={(_, d) => setBits(Number(d.value))} disabled={busy}>
                  {choices.map((b, i) => (
                    <option key={b} value={i}>
                      {b}
                    </option>
                  ))}
                </Select>
              </Field>
            )}
            <Field label="パスフレーズ（任意）">
              <Input type="password" value={passphrase} disabled={busy} onChange={(_, d) => setPassphrase(d.value)} />
            </Field>
            <Field
              label="パスフレーズの確認"
              validationState={mismatch ? 'warning' : 'none'}
              validationMessage={mismatch ? 'パスフレーズが一致しません' : undefined}
            >
              <Input type="password" value={confirm} disabled={busy} onChange={(_, d) => setConfirm(d.value)} />
            </Field>
            <Field label="コメント（任意）">
              <Input value={comment} disabled={busy} onChange={(_, d) => setComment(d.value)} />
            </Field>
            <Field label="秘密鍵の保存先">
              <div className={styles.row}>
                <Input className={styles.grow} value={path} disabled={busy} onChange={(_, d) => setPath(d.value)} />
                <Button
                  disabled={busy}
                  onClick={async () => {
                    const picked = await api.keygenPickPath();
                    if (picked) setPath(picked);
                  }}
                >
                  参照
                </Button>
              </div>
            </Field>
            {exists && (
              <Checkbox
                checked={overwrite}
                onChange={(_, d) => setOverwrite(d.checked === true)}
                label="既存の秘密鍵ファイルを上書きする"
              />
            )}
            <Checkbox
              checked={register}
              disabled={!registerSession || busy}
              onChange={(_, d) => setRegister(d.checked === true)}
              label={registerSession ? '開いている接続先に公開鍵を登録する' : '公開鍵の登録（接続中のSSHがありません）'}
            />
            <Caption1>OpenSSH形式で保存し、Unixでは権限を600にします。</Caption1>
            {busy && <Spinner size="tiny" label="生成しています…" />}
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
            {result && (
              <>
                <MessageBar intent="success">
                  <MessageBarBody>
                    秘密鍵を保存しました: {result.path}
                    {result.registered === true && '（公開鍵を接続先に登録しました）'}
                    {result.registered === false && '（接続先には同じ公開鍵が登録済みでした）'}
                  </MessageBarBody>
                </MessageBar>
                {result.warning && (
                  <MessageBar intent="warning">
                    <MessageBarBody>{result.warning}</MessageBarBody>
                  </MessageBar>
                )}
                <Field label="公開鍵">
                  <Textarea textarea={{ className: styles.mono }} readOnly rows={3} value={result.publicLine} />
                </Field>
                <div className={styles.row}>
                  <Button onClick={() => void navigator.clipboard?.writeText(result.publicLine)}>公開鍵をコピー</Button>
                  <Button onClick={() => void navigator.clipboard?.writeText(result.path)}>秘密鍵のパスをコピー</Button>
                </div>
              </>
            )}
          </DialogContent>
          <DialogActions>
            <Button disabled={busy} onClick={onClose}>
              閉じる
            </Button>
            <Button appearance="primary" disabled={!ready} onClick={() => void generate()}>
              生成
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
