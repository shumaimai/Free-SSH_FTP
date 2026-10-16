// 外部連携。MCPクライアント（Claude Desktopなど）へ、選んだ端末の閲覧・操作を15分だけ許可する。
// 許可はメモリ内だけで、停止・選び直しで進行中の要求も取り消される。操作の承認はRust側が画面へ出す。
import { useCallback, useEffect, useState } from 'react';
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
  Divider,
  MessageBar,
  MessageBarBody,
  Radio,
  RadioGroup,
  Text,
  Textarea,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { ExternalStatus, ShareMode } from './types';

const MODE_LABEL: Record<ShareMode, string> = {
  consult: '読むだけ',
  confirm: '操作ごとに確認',
  auto: '選択した端末で自動実行',
};

interface Props {
  api: Api;
  open: boolean;
  /** 接続できている端末。 */
  targets: { sessionId: string; label: string }[];
  onClose(): void;
}

export function ExternalDialog({ api, open, targets, onClose }: Props) {
  const [status, setStatus] = useState<ExternalStatus | null>(null);
  const [mode, setMode] = useState<ShareMode>('consult');
  const [selected, setSelected] = useState<string[]>([]);
  const [error, setError] = useState('');
  const [info, setInfo] = useState('');
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    void api
      .externalStatus()
      .then(setStatus)
      .catch((e) => setError(String(e)));
  }, [api]);

  useEffect(() => {
    if (!open) return;
    setError('');
    setInfo('');
    refresh();
    const timer = window.setInterval(refresh, 2000);
    return () => window.clearInterval(timer);
  }, [open, refresh]);

  const run = async (task: () => Promise<ExternalStatus>, done: string) => {
    setBusy(true);
    setError('');
    setInfo('');
    try {
      setStatus(await task());
      setInfo(done);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const live = targets.filter((t) => selected.includes(t.sessionId)).map((t) => t.sessionId);
  const sharing = status ? status.shared > 0 : false;

  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface style={{ maxWidth: 'min(620px, calc(100vw - 32px))' }}>
        <DialogBody>
          <DialogTitle>外部連携（MCP）</DialogTitle>
          <DialogContent style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Text>MCPクライアント（Claude Desktopなど）に、端末の閲覧・操作を許可します。</Text>
            <Caption1>
              アプリ内のAIに見せる端末は、AIパネルで選びます。端末の出力は信頼できない観測データで、秘密の除去は完全ではありません。
            </Caption1>
            <RadioGroup value={mode} onChange={(_, d) => setMode(d.value as ShareMode)}>
              {(Object.keys(MODE_LABEL) as ShareMode[]).map((m) => (
                <Radio key={m} value={m} label={MODE_LABEL[m]} />
              ))}
            </RadioGroup>
            <Text weight="semibold">共有する端末</Text>
            {targets.length === 0 && <Caption1>接続できている端末がありません。</Caption1>}
            {targets.map((t) => (
              <Checkbox
                key={t.sessionId}
                label={t.label}
                checked={selected.includes(t.sessionId)}
                onChange={(_, d) =>
                  setSelected((old) =>
                    d.checked ? [...old.filter((id) => id !== t.sessionId), t.sessionId] : old.filter((id) => id !== t.sessionId),
                  )
                }
              />
            ))}
            <Caption1>共有は15分間です。端末を選び直すか停止すると、実行許可も取り消します。</Caption1>
            <div style={{ display: 'flex', gap: 8 }}>
              <Button
                appearance="primary"
                disabled={busy || live.length === 0}
                onClick={() => void run(() => api.externalShare(mode, live), `${live.length}端末の共有を開始しました`)}
              >
                共有を開始
              </Button>
              <Button disabled={busy || !sharing} onClick={() => void run(() => api.externalStop(), '外部操作の共有を停止しました')}>
                停止
              </Button>
            </div>
            {status && (
              <Caption1>
                {status.shared}端末を共有中 ·{' '}
                {status.mode === 'confirm' ? '操作ごとに確認' : status.mode === 'auto' ? '自動実行' : '読むだけ'} · 残り
                {Math.floor(status.seconds / 60)}分
                {status.active > 0 ? ` · 実行中・承認待ち ${status.active}件` : ''}
              </Caption1>
            )}
            <Divider />
            <Text weight="semibold">MCP（ローカル）</Text>
            <Caption1>起動中のこのHashiへだけ接続できるstdioサーバーです。外部へ公開しません。</Caption1>
            {status && !status.mcpRunning && (
              <div>
                <Button disabled={busy || status.mcpStarting} onClick={() => void run(() => api.mcpStart(), 'MCPを開始しました')}>
                  MCPを開始
                </Button>
              </div>
            )}
            {status?.mcpRunning && (
              <>
                <Caption1>MCPクライアントの設定（このHashiを終了すると無効になります）</Caption1>
                <Textarea readOnly rows={5} value={status.mcpConfig} style={{ fontFamily: 'Consolas, monospace' }} />
                <div style={{ display: 'flex', gap: 8 }}>
                  <Button
                    onClick={() =>
                      void navigator.clipboard
                        .writeText(status.mcpConfig)
                        .then(() => setInfo('設定をコピーしました'))
                        .catch(() => setError('コピーできませんでした。選択して手動でコピーしてください'))
                    }
                  >
                    設定をコピー
                  </Button>
                  <Button disabled={busy} onClick={() => void run(() => api.mcpStop(), 'MCPを停止しました')}>
                    MCPを停止
                  </Button>
                </div>
              </>
            )}
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
            <Button onClick={onClose}>閉じる</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
