import { useEffect, useRef, useState } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import {
  Button,
  Caption1,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  makeStyles,
  mergeClasses,
  tokens,
} from '@fluentui/react-components';
import type { Api } from './api';
import type { SessionInfo } from './types';
import { InputQueue } from './inputQueue';
import { t } from './i18n';
import { TERM_FONTS, termTheme, type Appearance } from './appearance';

const useStyles = makeStyles({
  host: {
    position: 'absolute',
    inset: 0,
    padding: '10px 4px 6px 14px',
    boxSizing: 'border-box',
  },
  hidden: { visibility: 'hidden', pointerEvents: 'none' },
  pastePreview: {
    fontFamily: 'Consolas, "Cascadia Mono", monospace',
    fontSize: '13px',
    whiteSpace: 'pre-wrap',
    overflowWrap: 'anywhere',
    maxHeight: '240px',
    overflowY: 'auto',
    margin: 0,
    padding: '8px 10px',
    borderRadius: tokens.borderRadiusMedium,
    backgroundColor: tokens.colorNeutralBackground3,
  },
});

/** 貼り付ける文字から、端末の操作として働く制御文字(ESC・C1など)を除く。改行・タブは残す。
 *  ESCが残ると「貼り付けモードの終わり」を偽装して、貼り付けた途中でコマンドを実行できてしまう。 */
export function cleanPaste(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f-\u009f]/g, '');
}

interface Props {
  api: Api;
  active: boolean;
  dark: boolean;
  appearance: Appearance;
  /** 端末の大きさが決まってから呼ばれる。出力はsinkへ渡す。 */
  start: (
    size: { cols: number; rows: number },
    sink: (bytes: Uint8Array) => void,
  ) => Promise<SessionInfo>;
  onOpened: (info: SessionInfo) => void;
  onError: (message: string) => void;
}

const fontStack = (id: string) => (TERM_FONTS.find((f) => f.id === id) ?? TERM_FONTS[0]).stack;

export function TerminalView({ api, active, dark, appearance, start, onOpened, onError }: Props) {
  const styles = useStyles();
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const idRef = useRef<string | null>(null);
  // 複数行の貼り付けは、確認してから端末へ送る。
  const [pasteAsk, setPasteAsk] = useState<string | null>(null);
  // 親の再描画で関数が変わっても、マウント時の1回だけ開始する。
  const latest = useRef({ start, onOpened, onError, dark, appearance });
  latest.current = { start, onOpened, onError, dark, appearance };

  // 外観の変更は、作り直さずに反映する。フォントが変わると1文字の大きさも変わるので、列数・行数を取り直して伝える。
  useEffect(() => {
    const terminal = term.current;
    if (!terminal) return;
    terminal.options.theme = termTheme(appearance.termScheme, dark, appearance.accent);
    terminal.options.fontFamily = fontStack(appearance.termFont);
    terminal.options.fontSize = appearance.termSize;
    const timer = window.setTimeout(() => {
      fitRef.current?.fit();
      const id = idRef.current;
      if (id) void api.resize(id, terminal.cols, terminal.rows).catch(() => {});
    }, 30);
    return () => window.clearTimeout(timer);
  }, [api, dark, appearance]);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const first = latest.current.appearance;
    const terminal = new Terminal({
      fontFamily: fontStack(first.termFont),
      fontSize: first.termSize,
      cursorBlink: true,
      scrollback: 5000,
      theme: termTheme(first.termScheme, latest.current.dark, first.accent),
    });
    term.current = terminal;
    const fit = new FitAddon();
    fitRef.current = fit;
    terminal.loadAddon(fit);
    terminal.open(element);
    // 貼り付けは端末より先に受け取り、制御文字を除いてから送る。複数行なら確認する。
    const onPaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData('text/plain') ?? '';
      event.preventDefault();
      event.stopImmediatePropagation();
      const clean = cleanPaste(text);
      if (!clean) return;
      if (clean.replace(/\r\n?/g, '\n').replace(/\n+$/, '').includes('\n')) setPasteAsk(clean);
      else terminal.paste(clean);
    };
    element.addEventListener('paste', onPaste, true);
    fit.fit();
    let id: string | null = null;
    let alive = true;
    const queue = new InputQueue(
      (data) => (id ? api.input(id, data) : Promise.resolve()),
      (message) => latest.current.onError(message),
    );
    const input = terminal.onData((data) => queue.push(data));
    let timer: number | undefined;
    const observer = new ResizeObserver(() => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => {
        if (element.clientWidth === 0 || element.clientHeight === 0) return;
        fit.fit();
        if (id) void api.resize(id, terminal.cols, terminal.rows).catch(() => {});
      }, 40);
    });
    observer.observe(element);
    latest.current
      .start({ cols: terminal.cols, rows: terminal.rows }, (bytes) => {
        if (alive) terminal.write(bytes);
      })
      .then((info) => {
        id = info.id;
        idRef.current = info.id;
        if (alive) latest.current.onOpened(info);
      })
      .catch((error) => latest.current.onError(String(error)));
    return () => {
      alive = false;
      window.clearTimeout(timer);
      observer.disconnect();
      element.removeEventListener('paste', onPaste, true);
      input.dispose();
      terminal.dispose();
      term.current = null;
      fitRef.current = null;
      idRef.current = null;
    };
    // 端末は1つの画面につき1回だけ作る。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (active) {
      term.current?.focus();
    }
  }, [active]);

  const lines = pasteAsk ? pasteAsk.replace(/\r\n?/g, '\n').replace(/\n+$/, '').split('\n').length : 0;
  return (
    <>
      <div
        ref={host}
        className={mergeClasses(styles.host, !active && styles.hidden)}
        style={{ backgroundColor: termTheme(appearance.termScheme, dark, appearance.accent).background }}
      />
      <Dialog open={pasteAsk !== null} onOpenChange={(_, d) => !d.open && setPasteAsk(null)}>
        <DialogSurface aria-describedby={undefined} style={{ maxWidth: 560 }}>
          <DialogBody>
            <DialogTitle>{t('{lines}行を貼り付けますか？', { lines })}</DialogTitle>
            <DialogContent style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
              <Caption1>{t('各行がコマンドとして実行されることがあります。内容を確認してください。')}</Caption1>
              <pre className={styles.pastePreview}>{pasteAsk}</pre>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setPasteAsk(null)}>{t('やめる')}</Button>
              <Button
                appearance="primary"
                onClick={() => {
                  const text = pasteAsk;
                  setPasteAsk(null);
                  if (text) term.current?.paste(text);
                  term.current?.focus();
                }}
              >
                {t('貼り付ける')}
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </>
  );
}
