// 外部連携。MCPクライアント（Claude Desktopなど）へ、選んだ端末の閲覧・操作を、停止するまで許可する。
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
import { N_, t } from './i18n';
import type { ExternalStatus, RegisterStatus, ShareMode } from './types';

const REGISTER_STATE: Record<RegisterStatus['desktopState'], string> = {
  registered: N_('登録済みです。Claude Desktopを起動し直すと使えます。'),
  notRegistered: N_('未登録です。'),
  otherPath: N_('別の場所のHashiが登録されています。登録すると今のHashiへ更新します。'),
  unreadable: N_('設定ファイルを読めないため、上書きしません。下の「手動で登録する」を使ってください。'),
  noConfig: N_('Claude Desktopの設定フォルダが見つかりません。一度Claude Desktopを起動してください。'),
};

const MODE_LABEL: Record<ShareMode, string> = {
  consult: N_('読むだけ'),
  confirm: N_('操作ごとに確認'),
  auto: N_('選択した端末で自動実行'),
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
      .catch(() => setError(t('コピーできませんでした。選択して手動でコピーしてください')));

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

  const live = targets.filter((x) => selected.includes(x.sessionId)).map((x) => x.sessionId);
  const sharing = status ? status.shared > 0 : false;
  // 共有中(MCP起動中)は設定を固定する。変えるには停止して、設定し直してから開始する。
  const locked = sharing || (status?.mcpRunning ?? false);

  return (
    <Dialog open={open} onOpenChange={(_, d) => !d.open && onClose()}>
      <DialogSurface style={{ maxWidth: 'min(620px, calc(100vw - 32px))' }}>
        <DialogBody>
          <DialogTitle>{t('外部連携（MCP）')}</DialogTitle>
          <DialogContent style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            <Text>{t('MCPクライアント（Claude Desktopなど）に、端末の閲覧・操作を許可します。')}</Text>
            <Caption1>
              {t('アプリ内のAIに見せる端末は、AIパネルで選びます。端末の出力は信頼できない観測データで、秘密の除去は完全ではありません。')}
            </Caption1>
            <RadioGroup value={locked && status ? status.mode : mode} disabled={locked} onChange={(_, d) => setMode(d.value as ShareMode)}>
              {(Object.keys(MODE_LABEL) as ShareMode[]).map((m) => (
                <Radio key={m} value={m} label={t(MODE_LABEL[m])} />
              ))}
            </RadioGroup>
            <Text weight="semibold">{t('共有する端末')}</Text>
            {targets.length === 0 && <Caption1>{t('接続できている端末がありません。')}</Caption1>}
            {targets.map((x) => (
              <Checkbox
                key={x.sessionId}
                label={x.label}
                checked={selected.includes(x.sessionId)}
                disabled={locked}
                onChange={(_, d) =>
                  setSelected((old) =>
                    d.checked ? [...old.filter((id) => id !== x.sessionId), x.sessionId] : old.filter((id) => id !== x.sessionId),
                  )
                }
              />
            ))}
            <Caption1>
              {t('共有を開始するとMCPも始まります。期限はありません。共有中は設定を変えられません。変えるときは停止して、設定し直してから開始してください。')}
            </Caption1>
            <div style={{ display: 'flex', gap: 8 }}>
              <Button
                appearance="primary"
                disabled={busy || locked || live.length === 0}
                onClick={() => void run(() => api.externalShare(mode, live), t('{n}端末の共有を開始しました', { n: live.length }))}
              >
                {t('共有を開始')}
              </Button>
              <Button disabled={busy || !(sharing || status?.mcpRunning)} onClick={() => void run(() => api.externalStop(), t('共有とMCPを停止しました'))}>
                {t('停止')}
              </Button>
            </div>
            {status && (
              <Caption1>
                {[
                  t('{n}端末を共有中', { n: status.shared }),
                  status.mode === 'confirm' ? t('操作ごとに確認') : status.mode === 'auto' ? t('自動実行') : t('読むだけ'),
                  status.seconds === null ? t('期限なし') : t('残り{n}分', { n: Math.floor(status.seconds / 60) }),
                  ...(status.active > 0 ? [t('実行中・承認待ち {n}件', { n: status.active })] : []),
                ].join(' · ')}
              </Caption1>
            )}
            <Divider />
            <Text weight="semibold">{t('Claudeに登録（最初の1回だけ）')}</Text>
            <Caption1>
              {t('登録するのはHashiのヘルパーの場所だけです。Hashiを再起動しても、登録し直す必要はありません。')}
            </Caption1>
            {reg && !reg.helperFound && (
              <MessageBar intent="warning" layout="multiline" style={{ flexShrink: 0 }}>
                <MessageBarBody>
                  {t('HashiMCP-RustPreview.exeが、Hashi本体と同じフォルダにありません。同梱のフォルダのまま使ってください。')}
                </MessageBarBody>
              </MessageBar>
            )}
            {reg && (
              <>
                <Text>Claude Desktop</Text>
                <Caption1>{t(REGISTER_STATE[reg.desktopState])}</Caption1>
                <MessageBar intent="warning" layout="multiline" style={{ flexShrink: 0 }}>
                  <MessageBarBody>
                    {t('登録の前に、Claude Desktopを完全に終了してください（タスクトレイのアイコンを右クリックして「終了」）。起動中に登録すると、Claude Desktopが設定を保存し直すときに消えます。')}
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
                              ? t('登録しました。元の設定は {backup} に残しました。Claude Desktopを起動し直してください', { backup: next.backup })
                              : t('登録しました。Claude Desktopを起動し直してください'),
                          );
                        })
                        .catch((e) => setError(String(e)))
                        .finally(() => setBusy(false));
                    }}
                  >
                    {t('Claude Desktopに登録')}
                  </Button>
                </div>
                <Text>Claude Code</Text>
                <Caption1>{t('ターミナルに貼って実行してください。')}</Caption1>
                <Textarea readOnly rows={2} value={reg.codeCommand} style={{ fontFamily: 'Consolas, monospace' }} />
                <div>
                  <Button onClick={() => copy(reg.codeCommand, t('コマンドをコピーしました'))}>{t('コマンドをコピー')}</Button>
                </div>
                <Accordion collapsible>
                  <AccordionItem value="manual">
                    <AccordionHeader size="small">{t('手動で登録する')}</AccordionHeader>
                    <AccordionPanel>
                      <Caption1>
                        {t('MCPクライアントの設定ファイルの「mcpServers」に、次の項目を加えてください。ほかの項目は変えないでください。')}
                      </Caption1>
                      <Textarea readOnly rows={6} value={reg.manualConfig} style={{ fontFamily: 'Consolas, monospace', width: '100%' }} />
                      <div style={{ marginTop: 6 }}>
                        <Button onClick={() => copy(reg.manualConfig, t('設定をコピーしました'))}>{t('設定をコピー')}</Button>
                      </div>
                    </AccordionPanel>
                  </AccordionItem>
                </Accordion>
              </>
            )}
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
            <Button onClick={onClose}>{t('閉じる')}</Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
