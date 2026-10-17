import { useEffect, useState } from 'react';
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
  Field,
  Input,
  makeStyles,
  MessageBar,
  MessageBarBody,
  Select,
  tokens,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { ForwardInfo } from './types';

const useStyles = makeStyles({
  form: { display: 'flex', flexDirection: 'column', gap: '12px' },
  row: { display: 'grid', gridTemplateColumns: 'minmax(0,1fr) 120px', gap: '10px', '> *': { minWidth: 0 } },
  item: { display: 'flex', alignItems: 'center', gap: '8px' },
  grow: { flex: 1, minWidth: 0, overflowWrap: 'anywhere' },
  note: { color: tokens.colorNeutralForeground3 },
});

interface Props {
  api: Api;
  open: boolean;
  /** 対象のSSH接続。なければnull。 */
  sessionId: string | null;
  label: string;
  onClose(): void;
}

export function ForwardDialog({ api, open, sessionId, label, onClose }: Props) {
  const styles = useStyles();
  const [kind, setKind] = useState('L');
  const [bind, setBind] = useState('127.0.0.1');
  const [port, setPort] = useState('8080');
  const [host, setHost] = useState('127.0.0.1');
  const [targetPort, setTargetPort] = useState('22');
  const [list, setList] = useState<ForwardInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (open && sessionId) void api.forwardList(sessionId).then(setList).catch(() => setList([]));
    setError(null);
  }, [open, sessionId, api]);

  const start = async () => {
    if (!sessionId) return;
    setBusy(true);
    setError(null);
    try {
      setList(await api.forwardStart(sessionId, kind, bind, Number(port), host, Number(targetPort)));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 520 }}>
        <DialogBody>
          <DialogTitle>ポート転送</DialogTitle>
          <DialogContent className={styles.form}>
            {!sessionId ? (
              <Body1>SSHで接続した画面を開いてから使います。</Body1>
            ) : (
              <>
                <Caption1>対象: {label}</Caption1>
                <Field label="種類">
                  <Select value={kind} onChange={(_, d) => setKind(d.value)}>
                    <option value="L">ローカル -L（自分のPCの待受 → 接続先経由）</option>
                    <option value="R">リモート -R（接続先の待受 → 自分のPC側）</option>
                    <option value="D">SOCKS5 -D（接続先経由のプロキシ）</option>
                  </Select>
                </Field>
                <div className={styles.row}>
                  <Field label="待受アドレス（公開範囲を確認してください）">
                    <Input value={bind} onChange={(_, d) => setBind(d.value)} />
                  </Field>
                  <Field label="待受ポート">
                    <Input type="number" style={{ width: "100%", minWidth: 0 }} value={port} onChange={(_, d) => setPort(d.value)} />
                  </Field>
                </div>
                {kind !== 'D' ? (
                  <div className={styles.row}>
                    <Field label="転送先ホスト">
                      <Input value={host} onChange={(_, d) => setHost(d.value)} />
                    </Field>
                    <Field label="転送先ポート">
                      <Input type="number" style={{ width: "100%", minWidth: 0 }} value={targetPort} onChange={(_, d) => setTargetPort(d.value)} />
                    </Field>
                  </div>
                ) : (
                  <Caption1 className={styles.note}>
                    SOCKS5は認証なしのCONNECTです。標準のloopback待受（127.0.0.1）を推奨します。
                  </Caption1>
                )}
                <Caption1 className={styles.note}>待受ポートを0にすると、空いているポートを自動で割り当てます。</Caption1>
                <div>
                  <Button appearance="primary" disabled={busy} onClick={() => void start()}>
                    開始
                  </Button>
                </div>
                {error && (
                  <MessageBar intent="error">
                    <MessageBarBody>{error}</MessageBarBody>
                  </MessageBar>
                )}
                {list.length > 0 && <Caption1>動作中の転送</Caption1>}
                {list.map((f) => (
                  <div key={f.id} className={styles.item}>
                    <span className={styles.grow}>{f.label}</span>
                    <Button
                      size="small"
                      onClick={() =>
                        sessionId &&
                        void api
                          .forwardStop(sessionId, f.id)
                          .then(setList)
                          .catch((e) => {
                            setError(String(e));
                            void api.forwardList(sessionId).then(setList);
                          })
                      }
                    >
                      停止
                    </Button>
                  </div>
                ))}
              </>
            )}
          </DialogContent>
          <DialogActions>
            <Button onClick={onClose}>閉じる</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
