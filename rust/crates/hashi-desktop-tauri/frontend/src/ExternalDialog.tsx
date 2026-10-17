// 外部連携。MCPクライアント（Claude Desktopなど）へ、選んだ端末の閲覧・操作を15分だけ許可する。
// 許可はメモリ内だけで、停止・選び直しで進行中の要求も取り消される。操作の承認はRust側が画面へ出す。
import { useCallback, useEffect, useState } from 'react';
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
  Divider,
  MessageBar,
  MessageBarBody,
  Radio,
  RadioGroup,
  Text,
  Textarea,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { ExternalStatus, RegisterStatus, ShareMode } from './types';

const REGISTER_STATE: Record<RegisterStatus['desktopState'], string> = {
  registered: '登録済みです。Claude Desktopを起動し直すと使えます。',
  notRegistered: '未登録です。',
  otherPath: '別の場所のHashiが登録されています。登録すると今のHashiへ更新します。',
  unreadable: '設定ファイルを読めないため、上書きしません。下の「手動で登録する」を使ってください。',
  noConfig: 'Claude Desktopの設定フォルダが見つかりません。一度Claude Desktopを起動してください。',
};

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
  const [reg, setReg] = useState<RegisterStatus | null>(null);
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
    void api
      .mcpRegisterStatus()
      .then(setReg)
      .catch((e) => setError(String(e)));
  }, [api]);

  const copy = (text: string, done: string) =>
    void navigator.clipboard
      .writeText(text)
      .then(() => setInfo(done))
      .catch(() => setError('コピーできませんでした。選択して手動でコピーしてください'));

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
            <Text weight="semibold">Claudeに登録（最初の1回だけ）</Text>
            <Caption1>
              登録するのはHashiのヘルパーの場所だけです。Hashiを再起動しても、登録し直す必要はありません。
            </Caption1>
            {reg && !reg.helperFound && (
              <MessageBar intent="warning" layout="multiline" style={{ flexShrink: 0 }}>
                <MessageBarBody>
                  HashiMCP-RustPreview.exeが、Hashi本体と同じフォルダにありません。同梱のフォルダのまま使ってください。
                </MessageBarBody>
              </MessageBar>
            )}
            {reg && (
              <>
                <Text>Claude Desktop</Text>
                <Caption1>{REGISTER_STATE[reg.desktopState]}</Caption1>
                <MessageBar intent="warning" layout="multiline" style={{ flexShrink: 0 }}>
                  <MessageBarBody>
                    登録の前に、Claude Desktopを完全に終了してください（タスクトレイのアイコンを右クリックして「終了」）。起動中に登録すると、Claude
                    Desktopが設定を保存し直すときに消えます。
                  </MessageBarBody>
                </MessageBar>
                <div>
                  <Button
                    appearance="primary"
                    disabled={busy || !reg.helperFound || reg.desktopState === 'noConfig' || reg.desktopState === 'unreadable'}
                    onClick={() => {
                      setBusy(true);
                      setError('');
                      setInfo('');
                      api
                        .mcpRegisterDesktop()
                        .then((next) => {
                          setReg(next);
                          setInfo(
                            next.backup
                              ? `登録しました。元の設定は ${next.backup} に残しました。Claude Desktopを起動し直してください`
                              : '登録しました。Claude Desktopを起動し直してください',
                          );
                        })
                        .catch((e) => setError(String(e)))
                        .finally(() => setBusy(false));
                    }}
                  >
                    Claude Desktopに登録
                  </Button>
                </div>
                <Text>Claude Code</Text>
                <Caption1>ターミナルに貼って実行してください。</Caption1>
                <Textarea readOnly rows={2} value={reg.codeCommand} style={{ fontFamily: 'Consolas, monospace' }} />
                <div>
                  <Button onClick={() => copy(reg.codeCommand, 'コマンドをコピーしました')}>コマンドをコピー</Button>
                </div>
                <Accordion collapsible>
                  <AccordionItem value="manual">
                    <AccordionHeader size="small">手動で登録する</AccordionHeader>
                    <AccordionPanel>
                      <Caption1>
                        MCPクライアントの設定ファイルの「mcpServers」に、次の項目を加えてください。ほかの項目は変えないでください。
                      </Caption1>
                      <Textarea readOnly rows={6} value={reg.manualConfig} style={{ fontFamily: 'Consolas, monospace', width: '100%' }} />
                      <div style={{ marginTop: 6 }}>
                        <Button onClick={() => copy(reg.manualConfig, '設定をコピーしました')}>設定をコピー</Button>
                      </div>
                    </AccordionPanel>
                  </AccordionItem>
                </Accordion>
              </>
            )}
            <Divider />
            <Text weight="semibold">MCPの接続</Text>
            <Caption1>
              「MCPを開始」すると、登録済みのClaudeから使えます。起動中のこのHashiへだけ接続でき、外部へ公開しません。Hashiを終了すると無効になります。
            </Caption1>
            <div>
              {status && !status.mcpRunning && (
                <Button disabled={busy || status.mcpStarting} onClick={() => void run(() => api.mcpStart(), 'MCPを開始しました')}>
                  MCPを開始
                </Button>
              )}
              {status?.mcpRunning && (
                <Button disabled={busy} onClick={() => void run(() => api.mcpStop(), 'MCPを停止しました')}>
                  MCPを停止
                </Button>
              )}
            </div>
            {error && (
              <MessageBar intent="error" style={{ flexShrink: 0 }}>
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
            {info && (
              <MessageBar intent="success" style={{ flexShrink: 0 }}>
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
