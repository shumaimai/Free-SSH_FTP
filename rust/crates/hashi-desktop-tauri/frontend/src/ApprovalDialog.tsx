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
import type { ApprovalRequest } from './types';

const ACTION: Record<ApprovalRequest['action'], string> = {
  send_input: '対話端末へ入力',
  run_command: '独立コマンドを実行',
  interrupt: 'Ctrl+Cを送信',
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
  if (!request) return null;
  return (
    <Dialog open modalType="alert">
      <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
        <DialogBody>
          <DialogTitle>外部からの端末操作の確認</DialogTitle>
          <DialogContent className={styles.body}>
            <MessageBar intent="warning">
              <MessageBarBody>内容を確認してから許可してください。</MessageBarBody>
            </MessageBar>
            <Body1>依頼元: {request.actor}</Body1>
            <Body1>操作: {ACTION[request.action]}</Body1>
            <Body1>
              端末: {request.label}（世代{request.generation}）
            </Body1>
            {request.cwd && <Body1>開始フォルダ: {request.cwd}</Body1>}
            {request.text && (
              <>
                <Caption1>内容</Caption1>
                <pre className={styles.code}>{request.text}</pre>
                {request.action === 'send_input' && (
                  <Caption1>
                    {request.text.endsWith('\n') ? '入力のあと、Enterも送信します。' : 'Enterは送信しません(文字を打つだけです)。'}
                  </Caption1>
                )}
              </>
            )}
          </DialogContent>
          <DialogActions>
            <Button onClick={() => onAnswer(false)}>拒否</Button>
            <Button appearance="primary" onClick={() => onAnswer(true)}>
              許可して実行
            </Button>
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
