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
  MessageBarTitle,
  tokens,
} from '@fluentui/react-components';
import type { Challenge, ChallengeAnswer } from './types';

const useStyles = makeStyles({
  body: { display: 'flex', flexDirection: 'column', gap: '10px' },
  mono: {
    fontFamily: 'Consolas, "Cascadia Mono", monospace',
    fontSize: '13px',
    wordBreak: 'break-all',
    padding: '6px 8px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground3,
  },
});

interface Props {
  challenge: Challenge | null;
  error: string | null;
  onAnswer(answer: ChallengeAnswer): void;
}

/** ホスト鍵の確認と秘密入力。ここでの応答は、Rust側が控えた要求と照合される。閉じる操作は取消として扱う。 */
export function ChallengeDialog({ challenge, error, onAnswer }: Props) {
  const styles = useStyles();
  const [text, setText] = useState('');
  useEffect(() => setText(''), [challenge?.challenge_id]);
  if (!challenge) return null;
  const { scope } = challenge;
  const target = `${scope.username}@${scope.host}:${scope.port}${scope.hop !== null ? `（踏み台${scope.hop + 1}）` : ''}`;
  const body = challenge.challenge;
  return (
    <Dialog open modalType="alert">
      <DialogSurface aria-describedby={undefined}>
        <DialogBody>
          <DialogTitle>{target}</DialogTitle>
          <DialogContent className={styles.body}>
            {body.type === 'host_key' ? (
              <>
                {body.old ? (
                  <MessageBar intent="error">
                    <MessageBarBody>
                      <MessageBarTitle>このサーバーの鍵が、前に信頼した鍵と違います</MessageBarTitle>
                      サーバーの入れ替えか、通信の乗っ取りの可能性があります。心当たりがなければ中止してください。
                    </MessageBarBody>
                  </MessageBar>
                ) : (
                  <Body1>初めて接続するサーバーです。管理者から聞いた指紋と同じか確認してから信頼してください。</Body1>
                )}
                {body.old && (
                  <>
                    <Caption1>前に信頼した指紋</Caption1>
                    <div className={styles.mono}>{body.old.fingerprint}</div>
                  </>
                )}
                <Caption1>{body.old ? '今回の指紋' : '指紋'}（{body.key.key_type}）</Caption1>
                <div className={styles.mono}>{body.key.fingerprint}</div>
                {body.old && (
                  <Field label="信頼するには trust と入力してください">
                    <Input value={text} onChange={(_, d) => setText(d.value)} />
                  </Field>
                )}
              </>
            ) : (
              <Field label={body.prompt}>
                <Input
                  type="password"
                  autoFocus
                  value={text}
                  onChange={(_, d) => setText(d.value)}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' && !e.nativeEvent.isComposing)
                      onAnswer({ type: 'secret', value: text });
                  }}
                />
              </Field>
            )}
            {error && (
              <MessageBar intent="error">
                <MessageBarBody>{error}</MessageBarBody>
              </MessageBar>
            )}
          </DialogContent>
          <DialogActions>
            <Button onClick={() => onAnswer({ type: 'cancel' })}>中止</Button>
            {body.type === 'host_key' ? (
              <Button
                appearance="primary"
                disabled={body.old !== null && text !== 'trust'}
                onClick={() => onAnswer({ type: 'trust', confirmation: text })}
              >
                信頼して接続
              </Button>
            ) : (
              <Button appearance="primary" onClick={() => onAnswer({ type: 'secret', value: text })}>
                送信
              </Button>
            )}
          </DialogActions>
        </DialogBody>
      </DialogSurface>
    </Dialog>
  );
}
