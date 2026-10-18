// 画面の多言語化。文言の「原文」は日本語で、そのままキーにする。英語は src/i18n/*.ts の辞書から引く。
//   t('接続しました')                      … 日本語の原文。英語のときは辞書の訳に置き換わる
//   t('{n}件を削除', { n: 3 })            … {名前} を値で置き換える（日本語・英語どちらも）
//   N_('保存')                             … 表のラベルなど、定義時には訳さず後で t(label) と呼ぶ文言の印
// 辞書に無い文言は日本語のまま出す（画面が空になるより良い）。抜けは scripts/check-i18n.mjs が build で止める。
import { useSyncExternalStore } from 'react';
import { EN } from './i18n/en';
import { RUST_MESSAGES } from './i18n/rust';

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

// ---- Rust側から来る日本語のメッセージ ----
// エラー文などは Rust が日本語の文字列で返す。台帳(src/i18n/rust/*.json)に「原文の型 → 英訳」があり、
// 英語のときだけ引く。原文の `{}` `{name}` `{:?}` は任意の文字列に当たる穴で、訳の `{1}` `{2}`… に入る(穴の中身も再帰して訳す)。
// 台帳に無い文は日本語のまま返す。台帳の抜けは tools/rust_messages.py が CI で止める。
interface Template {
  pattern: RegExp;
  text: string;
  weight: number;
}

let exact: Map<string, string> | null = null;
let templates: Template[] = [];
let phrases: Array<[string, string]> = [];

const HOLE = /\{[^{}]*\}/g;
const escapeRe = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

function buildCatalog() {
  exact = new Map();
  templates = [];
  phrases = [];
  for (const [key, value] of Object.entries(RUST_MESSAGES)) {
    if (typeof value !== 'string') continue;
    const unbraced = key.replace(/\{\{/g, '\u0001').replace(/\}\}/g, '\u0002');
    const parts = unbraced.split(HOLE);
    const restore = (x: string) => x.replace(/\u0001/g, '{').replace(/\u0002/g, '}');
    if (parts.length === 1) {
      exact.set(restore(unbraced), value);
      phrases.push([restore(unbraced), value]);
    } else {
      const pattern = new RegExp('^' + parts.map((p) => escapeRe(restore(p))).join('([\\s\\S]*?)') + '$');
      templates.push({ pattern, text: value, weight: parts.join('').length });
    }
  }
  templates.sort((a, b) => b.weight - a.weight);
  phrases.sort((a, b) => b[0].length - a[0].length);
}

/** Rustが返した日本語のメッセージを、現在の言語の文言にする(日本語のときはそのまま)。 */
export function tr(message: string, depth = 0): string {
  if (current !== 'en' || depth > 4 || !/[\u3040-\u30ff\u3400-\u9fff]/.test(message)) return message;
  if (!exact) buildCatalog();
  const hit = exact!.get(message);
  if (hit !== undefined) return hit;
  for (const tpl of templates) {
    const m = tpl.pattern.exec(message);
    if (m) return tpl.text.replace(/\{(\d+)\}/g, (_, n: string) => tr(m[Number(n)] ?? '', depth + 1));
  }
  // 全体に当たらないときは、穴のない文を部分的に置き換える(`a: b` のように組み立てられた文のため)。
  let out = message;
  for (const [ja, en] of phrases) if (out.includes(ja)) out = out.split(ja).join(en);
  return out;
}
