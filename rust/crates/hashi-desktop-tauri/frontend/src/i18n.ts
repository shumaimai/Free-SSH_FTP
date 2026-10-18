// 画面の多言語化。文言の「原文」は日本語で、そのままキーにする。英語は src/i18n/*.ts の辞書から引く。
//   t('接続しました')                      … 日本語の原文。英語のときは辞書の訳に置き換わる
//   t('{n}件を削除', { n: 3 })            … {名前} を値で置き換える（日本語・英語どちらも）
//   N_('保存')                             … 表のラベルなど、定義時には訳さず後で t(label) と呼ぶ文言の印
// 辞書に無い文言は日本語のまま出す（画面が空になるより良い）。抜けは scripts/check-i18n.mjs が build で止める。
import { useSyncExternalStore } from 'react';
import { EN } from './i18n/en';

export type Lang = 'ja' | 'en';
export type LangSetting = 'auto' | Lang;

const STORAGE_KEY = 'hashi.language';

function readSetting(): LangSetting {
  try {
    const v = window.localStorage.getItem(STORAGE_KEY);
    if (v === 'ja' || v === 'en' || v === 'auto') return v;
  } catch {
    // 保存先が使えなくても動く。
  }
  return 'auto';
}

/** OSの言語が日本語なら日本語、それ以外は英語。 */
export function detectLang(): Lang {
  const languages = typeof navigator === 'undefined' ? [] : [...(navigator.languages ?? []), navigator.language];
  const first = languages.find((l) => !!l);
  return first && first.toLowerCase().startsWith('ja') ? 'ja' : 'en';
}

let setting: LangSetting = readSetting();
let current: Lang = setting === 'auto' ? detectLang() : setting;
const listeners = new Set<() => void>();

function apply() {
  current = setting === 'auto' ? detectLang() : setting;
  if (typeof document !== 'undefined') document.documentElement.lang = current;
}
apply();

export function setLanguage(next: LangSetting) {
  setting = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // 保存できなくても、このセッションでは切り替わる。
  }
  apply();
  listeners.forEach((l) => l());
}

/** 言語が変わったら再描画する。設定を持つ最上位(App)と設定ダイアログで使う。 */
export function useLanguage(): { setting: LangSetting; lang: Lang } {
  useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => current,
  );
  return { setting, lang: current };
}

export function lang(): Lang {
  return current;
}

type Params = Record<string, string | number>;

function fill(text: string, params?: Params): string {
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in params ? String(params[name]) : whole));
}

/** 日本語の原文を、現在の言語の文言にする。 */
export function t(ja: string, params?: Params): string {
  const text = current === 'en' ? (EN[ja] ?? ja) : ja;
  return fill(text, params);
}

/** 後で t(label) と呼ぶ文言の印（実行時は何もしない）。check-i18n がキーとして数える。 */
export function N_(ja: string): string {
  return ja;
}
