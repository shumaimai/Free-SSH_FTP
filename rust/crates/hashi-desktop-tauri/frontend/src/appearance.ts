// 外観の候補。保存するのはここのidだけで、色やフォント名そのものは保存しない（Rust側も同じidだけ受け付ける）。
import { createDarkTheme, createLightTheme, type BrandVariants, type Theme } from '@fluentui/react-components';
import type { ITheme } from '@xterm/xterm';

export type ThemeMode = 'system' | 'light' | 'dark';

export interface Appearance {
  mode: ThemeMode;
  accent: string;
  uiFont: string;
  termFont: string;
  termSize: number;
  termScheme: string;
}

export const DEFAULT_APPEARANCE: Appearance = {
  mode: 'dark',
  accent: 'mint',
  uiFont: 'system',
  termFont: 'cascadia',
  termSize: 14,
  termScheme: 'auto',
};

export const MODES: { id: ThemeMode; label: string }[] = [
  { id: 'system', label: 'システムに合わせる' },
  { id: 'light', label: 'ライト' },
  { id: 'dark', label: 'ダーク' },
];

export const ACCENTS: { id: string; label: string; color: string }[] = [
  { id: 'mint', label: 'ミント（最初の画面）', color: '#56ccb2' },
  { id: 'teal', label: 'ティール', color: '#0f8b7d' },
  { id: 'blue', label: 'ブルー', color: '#0f6cbd' },
  { id: 'green', label: 'グリーン', color: '#2f8f46' },
  { id: 'purple', label: 'パープル', color: '#7a4bc7' },
  { id: 'rose', label: 'ローズ', color: '#c2385a' },
  { id: 'orange', label: 'オレンジ', color: '#c4651b' },
  { id: 'slate', label: 'グレー', color: '#52606d' },
];

export const UI_FONTS: { id: string; label: string; stack: string }[] = [
  {
    id: 'system',
    label: '標準（Segoe UI / 游ゴシック UI）',
    stack: '"Segoe UI Variable Text", "Segoe UI", "Yu Gothic UI", "Meiryo UI", system-ui, sans-serif',
  },
  { id: 'yugothic', label: '游ゴシック UI', stack: '"Yu Gothic UI", "Yu Gothic", "Meiryo UI", system-ui, sans-serif' },
  { id: 'meiryo', label: 'メイリオ UI', stack: '"Meiryo UI", "Meiryo", "Yu Gothic UI", system-ui, sans-serif' },
  { id: 'biz', label: 'BIZ UDPゴシック', stack: '"BIZ UDPGothic", "Yu Gothic UI", "Meiryo UI", system-ui, sans-serif' },
  { id: 'noto', label: 'Noto Sans JP', stack: '"Noto Sans JP", "Yu Gothic UI", "Meiryo UI", system-ui, sans-serif' },
];

export const TERM_FONTS: { id: string; label: string; stack: string }[] = [
  {
    id: 'cascadia',
    label: 'Cascadia Mono',
    stack: '"Cascadia Mono", "Cascadia Code", Consolas, "BIZ UDGothic", "MS Gothic", "Noto Sans Mono CJK JP", monospace',
  },
  { id: 'consolas', label: 'Consolas', stack: 'Consolas, "BIZ UDGothic", "MS Gothic", "Noto Sans Mono CJK JP", monospace' },
  { id: 'bizud', label: 'BIZ UDゴシック', stack: '"BIZ UDGothic", "MS Gothic", "Noto Sans Mono CJK JP", monospace' },
  { id: 'msgothic', label: 'MS ゴシック', stack: '"MS Gothic", "BIZ UDGothic", "Noto Sans Mono CJK JP", monospace' },
  { id: 'courier', label: 'Courier New', stack: '"Courier New", "BIZ UDGothic", "MS Gothic", monospace' },
];

interface Scheme {
  id: string;
  label: string;
  theme?: ITheme;
}

const ansi = (c: string[]): Partial<ITheme> => ({
  black: c[0], red: c[1], green: c[2], yellow: c[3], blue: c[4], magenta: c[5], cyan: c[6], white: c[7],
  brightBlack: c[8], brightRed: c[9], brightGreen: c[10], brightYellow: c[11], brightBlue: c[12],
  brightMagenta: c[13], brightCyan: c[14], brightWhite: c[15],
});

export const TERM_SCHEMES: Scheme[] = [
  { id: 'auto', label: '画面のテーマに合わせる' },
  {
    id: 'campbell',
    label: 'Campbell（Windows標準）',
    theme: {
      background: '#0c0c0c', foreground: '#cccccc', cursor: '#ffffff', selectionBackground: '#264f78',
      ...ansi(['#0c0c0c', '#c50f1f', '#13a10e', '#c19c00', '#0037da', '#881798', '#3a96dd', '#cccccc',
        '#767676', '#e74856', '#16c60c', '#f9f1a5', '#3b78ff', '#b4009e', '#61d6d6', '#f2f2f2']),
    },
  },
  {
    id: 'onedark',
    label: 'One Dark',
    theme: {
      background: '#282c34', foreground: '#abb2bf', cursor: '#528bff', selectionBackground: '#3e4451',
      ...ansi(['#282c34', '#e06c75', '#98c379', '#e5c07b', '#61afef', '#c678dd', '#56b6c2', '#abb2bf',
        '#5c6370', '#e06c75', '#98c379', '#e5c07b', '#61afef', '#c678dd', '#56b6c2', '#ffffff']),
    },
  },
  {
    id: 'solarized',
    label: 'Solarized Dark',
    theme: {
      background: '#002b36', foreground: '#93a1a1', cursor: '#93a1a1', selectionBackground: '#073642',
      ...ansi(['#073642', '#dc322f', '#859900', '#b58900', '#268bd2', '#d33682', '#2aa198', '#eee8d5',
        '#586e75', '#cb4b16', '#586e75', '#657b83', '#839496', '#6c71c4', '#93a1a1', '#fdf6e3']),
    },
  },
  {
    id: 'light',
    label: '明るい（紙のような白）',
    theme: {
      background: '#fbfbf9', foreground: '#2b2b2b', cursor: '#2b2b2b', selectionBackground: '#cfe3f5',
      ...ansi(['#2b2b2b', '#c0392b', '#1e8449', '#9a7d0a', '#1f5fa8', '#8e44ad', '#117a8b', '#d5d8dc',
        '#7f8c8d', '#e74c3c', '#27ae60', '#b7950b', '#2e86de', '#a569bd', '#16a5b8', '#ffffff']),
    },
  },
];

// ---- Fluent UIのテーマ生成 ----

function hexToHsl(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => v / 255);
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  const h = max === r ? (g - b) / d + (g < b ? 6 : 0) : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, s, l];
}

function hslToHex(h: number, s: number, l: number): string {
  const a = s * Math.min(l, 1 - l);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(255 * c).toString(16).padStart(2, '0');
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}

/** 基準色をFluentの16段階（10〜160）へ広げる。80番が基準色で、暗い側・明るい側へ線形に振る。 */
export function brandRamp(base: string): BrandVariants {
  const [h, s, l] = hexToHsl(base);
  const keys = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160] as const;
  const out: Record<number, string> = {};
  keys.forEach((key, i) => {
    const lightness = i <= 7 ? 0.08 + ((l - 0.08) * i) / 7 : l + ((0.96 - l) * (i - 7)) / 8;
    // 明るい側は彩度を少し落として、白に近い色が濁らないようにする。
    const saturation = i <= 7 ? s : s * (1 - 0.35 * ((i - 7) / 8));
    out[key] = hslToHex(h, Math.min(1, saturation), lightness);
  });
  return out as unknown as BrandVariants;
}

/** 最初のegui版（theme.rs）の配色・角丸。暗い画面ではFluentの無彩色ではなく、この青みのある暗色を使う。 */
const EGUI = {
  bg: '#111318',
  surface: '#181b21',
  card: '#1f232b',
  cardHover: '#282d38',
  border: '#303642',
  text: '#e6e9ef',
  muted: '#8992a3',
  danger: '#eb756c',
  warn: '#e6b85c',
  ok: '#62d28e',
  onAccent: '#0a1c18',
};

function eguiDark(base: Theme, accent: string): Theme {
  return {
    ...base,
    colorNeutralBackground1: EGUI.card,
    colorNeutralBackground1Hover: EGUI.cardHover,
    colorNeutralBackground1Pressed: EGUI.cardHover,
    colorNeutralBackground1Selected: EGUI.cardHover,
    colorNeutralBackground2: EGUI.bg,
    colorNeutralBackground3: EGUI.surface,
    colorNeutralBackground4: EGUI.bg,
    colorNeutralBackground5: '#0c0e12',
    colorNeutralBackground6: EGUI.card,
    colorSubtleBackgroundHover: EGUI.cardHover,
    colorSubtleBackgroundPressed: EGUI.cardHover,
    colorNeutralForeground1: EGUI.text,
    colorNeutralForeground1Hover: EGUI.text,
    colorNeutralForeground1Pressed: EGUI.text,
    colorNeutralForeground2: EGUI.text,
    colorNeutralForeground3: EGUI.muted,
    colorNeutralForeground4: EGUI.muted,
    colorNeutralStroke1: EGUI.border,
    colorNeutralStroke1Hover: accent,
    colorNeutralStroke2: EGUI.border,
    colorNeutralStroke3: EGUI.border,
    colorNeutralStrokeAccessible: EGUI.muted,
    colorPaletteRedForeground1: EGUI.danger,
    colorPaletteRedForeground3: EGUI.danger,
    colorPaletteYellowForeground1: EGUI.warn,
    colorPaletteGreenForeground1: EGUI.ok,
    colorBrandForeground1: accent,
    colorBrandForegroundLink: accent,
    colorBrandStroke1: accent,
    colorCompoundBrandStroke: accent,
    colorCompoundBrandForeground1: accent,
    colorBrandBackground: accent,
    colorBrandBackgroundHover: `color-mix(in srgb, ${accent} 88%, white)`,
    colorBrandBackgroundPressed: `color-mix(in srgb, ${accent} 80%, black)`,
    colorBrandBackground2: `color-mix(in srgb, ${accent} 22%, ${EGUI.card})`,
    colorBrandBackground2Hover: `color-mix(in srgb, ${accent} 30%, ${EGUI.card})`,
    colorNeutralForegroundOnBrand: EGUI.onAccent,
    borderRadiusMedium: '8px',
    borderRadiusLarge: '10px',
    borderRadiusXLarge: '12px',
    fontSizeBase300: '15px',
    lineHeightBase300: '22px',
    fontSizeBase200: '13px',
  };
}

export function fluentTheme(dark: boolean, accentId: string): Theme {
  const accent = ACCENTS.find((a) => a.id === accentId) ?? ACCENTS[0];
  const ramp = brandRamp(accent.color);
  if (!dark) return createLightTheme(ramp);
  return eguiDark(createDarkTheme(ramp), accent.color);
}

export function termTheme(schemeId: string, dark: boolean, accentId: string): ITheme {
  const scheme = TERM_SCHEMES.find((s) => s.id === schemeId);
  if (scheme?.theme) return scheme.theme;
  const accent = ACCENTS.find((a) => a.id === accentId) ?? ACCENTS[0];
  return dark
    ? { background: '#14171d', foreground: '#dadee6', cursor: accent.color, selectionBackground: '#2a4a48' }
    : { background: '#ffffff', foreground: '#1f1f1f', cursor: accent.color, selectionBackground: '#b7d7f0' };
}

export function sanitize(value: Partial<Appearance> | null | undefined): Appearance {
  const v = value ?? {};
  const pick = <T extends { id: string }>(list: T[], id: unknown, fallback: string) =>
    list.some((item) => item.id === id) ? (id as string) : fallback;
  const size = Number(v.termSize);
  return {
    mode: pick(MODES, v.mode, DEFAULT_APPEARANCE.mode) as ThemeMode,
    accent: pick(ACCENTS, v.accent, DEFAULT_APPEARANCE.accent),
    uiFont: pick(UI_FONTS, v.uiFont, DEFAULT_APPEARANCE.uiFont),
    termFont: pick(TERM_FONTS, v.termFont, DEFAULT_APPEARANCE.termFont),
    termSize: Number.isFinite(size) ? Math.min(24, Math.max(10, Math.round(size))) : DEFAULT_APPEARANCE.termSize,
    termScheme: pick(TERM_SCHEMES, v.termScheme, DEFAULT_APPEARANCE.termScheme),
  };
}
