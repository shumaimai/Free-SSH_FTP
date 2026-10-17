// 左右の幅を変えるつまみ。ドラッグ中の移動量だけを親へ渡し、値の範囲は親が決める。
// ダブルクリックで既定に戻す。キーボードでも左右キーで動かせる。
import { useRef, useState } from 'react';
import { makeStyles, mergeClasses, tokens } from '@fluentui/react-components';

const useStyles = makeStyles({
  grip: {
    width: '6px',
    flexShrink: 0,
    cursor: 'col-resize',
    touchAction: 'none',
    alignSelf: 'stretch',
    backgroundColor: 'transparent',
    transitionProperty: 'background-color',
    transitionDuration: tokens.durationFaster,
    ':hover': { backgroundColor: tokens.colorBrandBackground2 },
    ':focus-visible': { outline: `2px solid ${tokens.colorStrokeFocus2}`, outlineOffset: '-2px' },
  },
  active: { backgroundColor: tokens.colorBrandBackground2 },
});

interface Props {
  label: string;
  /** ドラッグ開始時の値を親が覚える。 */
  onBegin(): void;
  /** 開始位置からの横移動量(px)。 */
  onMove(dx: number): void;
  onReset(): void;
  className?: string;
}

export function Grip({ label, onBegin, onMove, onReset, className }: Props) {
  const styles = useStyles();
  const start = useRef<number | null>(null);
  const [dragging, setDragging] = useState(false);
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      title={`${label}(ダブルクリックで元に戻す)`}
      tabIndex={0}
      className={mergeClasses(styles.grip, dragging && styles.active, className)}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = e.clientX;
        setDragging(true);
        onBegin();
      }}
      onPointerMove={(e) => {
        if (start.current !== null) onMove(e.clientX - start.current);
      }}
      onPointerUp={(e) => {
        start.current = null;
        setDragging(false);
        if (e.currentTarget.hasPointerCapture(e.pointerId)) e.currentTarget.releasePointerCapture(e.pointerId);
      }}
      onPointerCancel={() => {
        start.current = null;
        setDragging(false);
      }}
      onDoubleClick={onReset}
      onKeyDown={(e) => {
        if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
        e.preventDefault();
        onBegin();
        onMove(e.key === 'ArrowLeft' ? -24 : 24);
      }}
    />
  );
}
