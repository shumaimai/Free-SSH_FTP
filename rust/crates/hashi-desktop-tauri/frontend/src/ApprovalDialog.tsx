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
  makeStyles,
  MessageBar,
  MessageBarBody,
  tokens,
} from '@fluentui/react-components';
import { useEffect, useState } from 'react';
import { N_, t } from './i18n';
import type { ApprovalRequest } from './types';

/** 内容が出てから「許可」を押せるようになるまでの時間。続けて届いた要求をダブルクリックで許可しないため。 */
const ARM_MS = 1000;

const ACTION: Record<ApprovalRequest['action'], string> = {
  send_input: N_('対話端末へ入力'),
  send_keys: N_('対話端末へキー送信'),
  run_command: N_('独立コマンドを実行'),
  interrupt: N_('Ctrl+Cを送信'),
};

const useStyles = makeStyles({
  body: { display: 'flex', flexDirection: 'column', gap: '8px' },
  code: {
    fontFamily: 'Consolas, "Cascadia Mono", monospace',
    fontSize: '13px',
    whiteSpace: 'pre-wrap',
    overflowWrap: 'anywhere',
    padding: '8px 10px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground3,
    margin: 0,
    maxHeight: '240px',
    overflowY: 'auto',
  },
});

/** 外部（AI/MCP）からの端末操作の許可。閉じる操作は「拒否」として扱う。 */
export function ApprovalDialog({ request, onAnswer }: { request: ApprovalRequest | null; onAnswer(allow: boolean): void }) {
  const styles = useStyles();
  const [armed, setArmed] = useState(false);
  const id = request?.id;
  useEffect(() => {
    setArmed(false);
    if (!id) return;
    const timer = window.setTimeout(() => setArmed(true), ARM_MS);
    return () => window.clearTimeout(timer);
  }, [id]);
  if (!request) return null;
  const lines = request.text ? request.text.replace(/\n$/, '').split('\n').length : 0;
  return (
    <Dialog open modalType="alert">
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
        <DialogBody>
          <DialogTitle>{t('外部からの端末操作の確認')}</DialogTitle>
          <DialogContent className={styles.body}>
            <MessageBar intent="warning">
              <MessageBarBody>{t('内容を確認してから許可してください。')}</MessageBarBody>
            </MessageBar>
            <Body1>{t('依頼元: {actor}', { actor: request.actor })}</Body1>
            <Body1>{t('操作: {action}', { action: t(ACTION[request.action]) })}</Body1>
            <Body1>
              {t('端末: {label}（世代{generation}）', { label: request.label, generation: request.generation })}
            </Body1>
            {request.cwd && <Body1>{t('開始フォルダ: {cwd}', { cwd: request.cwd })}</Body1>}
            {request.text && (
              <>
                <Caption1>
                  {t('内容（行数: {lines}、文字数: {chars}）。枠内をスクロールして最後まで確認してください', { lines, chars: request.text.length })}
                </Caption1>
                <pre className={styles.code}>{request.text}</pre>
                {request.action === 'send_input' && (
                  <Caption1>
                    {request.text.endsWith('\n') ? t('入力のあと、Enterも送信します。') : t('Enterは送信しません(文字を打つだけです)。')}
                    {request.text.slice(0, -1).includes('\n') && ' ' + t('途中の改行もEnterとして送るため、その行ごとに実行されます。')}
                  </Caption1>
                )}
              </>
            )}
          </DialogContent>
          <DialogActions>
            <Button onClick={() => onAnswer(false)}>{t('拒否')}</Button>
            <Button appearance="primary" disabled={!armed} onClick={() => onAnswer(true)}>
              {t('許可して実行')}
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
