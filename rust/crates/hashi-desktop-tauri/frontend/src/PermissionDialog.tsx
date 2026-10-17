// 権限補助（一時的な権限変更）。接続ごとに利用者が有効にしたときだけ、一覧・転送で権限が足りない
// 所有ファイルのmodeを一時的に緩め、作業後に元へ戻す。復元できなかったときは記録が残る。
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
  MessageBar,
  MessageBarBody,
  Text,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { PermissionStatus } from './types';

interface Props {
  api: Api;
  open: boolean;
  sessionId: string;
  onClose(): void;
}

export function PermissionDialog({ api, open, sessionId, onClose }: Props) {
  const [status, setStatus] = useState<PermissionStatus | null>(null);
  const [legacy, setLegacy] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [info, setInfo] = useState('');

  useEffect(() => {
    if (!open) return;
    setError('');
    setInfo('');
    setLegacy(false);
    void api
      .permissionsStatus(sessionId)
      .then(setStatus)
      .catch((e) => setError(String(e)));
  }, [open, api, sessionId]);

  const run = async (task: () => Promise<void>) => {
    setBusy(true);
    setError('');
    setInfo('');
    try {
      await task();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface style={{ maxWidth: 'min(620px, calc(100vw - 32px))' }}>
        <DialogBody>
          <DialogTitle>一時的な権限変更</DialogTitle>
          <DialogContent style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Text>一覧と転送で権限が足りないとき、自分が所有するファイルのmodeを一時的に変えて作業し、終わったら元へ戻します。</Text>
            <Caption1>
              復元に失敗したときは記録を残し、再接続後にここから復元できます。sudo・ACL・旧版（Python）の未復元記録の取り込みには対応していません。
              旧版に未復元の記録があるときは、有効にできません（旧版で復旧してください）。
            </Caption1>
            <Checkbox
              checked={legacy}
              onChange={(_, d) => setLegacy(Boolean(d.checked))}
              label="旧Python版をすべて終了した（確認できない旧版も含む）"
              disabled={status?.enabled}
            />
            <Text weight="semibold">この接続: {status ? (status.enabled ? '有効' : '無効') : '確認中'}</Text>
            {status?.note && <Caption1>{status.note}</Caption1>}
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
            {info && (
              <MessageBar intent="success">
                <MessageBarBody>{info}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            <Button
              appearance="primary"
              disabled={busy || !status || status.enabled || !legacy}
              onClick={() =>
                void run(async () => {
                  setStatus(await api.permissionsEnable(sessionId, legacy));
                  setInfo('この接続で有効にしました（接続を閉じるまで）');
                })
              }
            >
              この接続で有効にする
            </Button>
            <Button
              disabled={busy || !status?.enabled}
              onClick={() => void run(async () => setInfo(await api.permissionsRecover(sessionId)))}
            >
              この接続の未復元記録を復元
            </Button>
            <Button onClick={onClose}>閉じる</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
