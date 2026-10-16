import { useEffect, useRef } from 'react';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { makeStyles, mergeClasses } from '@fluentui/react-components';
import type { Api } from './api';
import type { SessionInfo } from './types';
import { InputQueue } from './inputQueue';

const useStyles = makeStyles({
  host: {
    position: 'absolute',
    inset: 0,
    padding: '10px 4px 6px 14px',
    boxSizing: 'border-box',
  },
  hidden: { visibility: 'hidden', pointerEvents: 'none' },
});

interface Props {
  api: Api;
  active: boolean;
  dark: boolean;
  /** 端末の大きさが決まってから呼ばれる。出力はsinkへ渡す。 */
  start: (
    size: { cols: number; rows: number },
    sink: (bytes: Uint8Array) => void,
  ) => Promise<SessionInfo>;
  onOpened: (info: SessionInfo) => void;
  onError: (message: string) => void;
}

const TERMINAL_FONT =
  '"Cascadia Mono", "Cascadia Code", Consolas, "BIZ UDGothic", "MS Gothic", "Noto Sans Mono CJK JP", monospace';

function palette(dark: boolean) {
  return dark
    ? { background: '#1b1b1f', foreground: '#e6e6e6', cursor: '#56ccb2', selectionBackground: '#3a5f7a' }
    : { background: '#ffffff', foreground: '#1f1f1f', cursor: '#0f7b6c', selectionBackground: '#b7d7f0' };
}

export function TerminalView({ api, active, dark, start, onOpened, onError }: Props) {
  const styles = useStyles();
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  // 親の再描画で関数が変わっても、マウント時の1回だけ開始する。
  const latest = useRef({ start, onOpened, onError, dark });
  latest.current = { start, onOpened, onError, dark };

  useEffect(() => {
    if (term.current) term.current.options.theme = palette(dark);
  }, [dark]);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    const terminal = new Terminal({
      fontFamily: TERMINAL_FONT,
      fontSize: 14,
      cursorBlink: true,
      scrollback: 5000,
      theme: palette(latest.current.dark),
    });
    term.current = terminal;
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(element);
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
        if (alive) latest.current.onOpened(info);
      })
      .catch((error) => latest.current.onError(String(error)));
    return () => {
      alive = false;
      window.clearTimeout(timer);
      observer.disconnect();
      input.dispose();
      terminal.dispose();
      term.current = null;
    };
    // 端末は1つの画面につき1回だけ作る。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (active) {
      term.current?.focus();
    }
  }, [active]);

  return <div
      ref={host}
      className={mergeClasses(styles.host, !active && styles.hidden)}
      style={{ backgroundColor: palette(dark).background }}
    />;
}
