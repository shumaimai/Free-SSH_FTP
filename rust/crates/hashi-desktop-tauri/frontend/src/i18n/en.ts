// 英語の辞書。キーは日本語の原文。ファイルごとに分けて持つ（src/i18n/en/ 以下）。
// 新しい文言は t('原文') を書いて、対応する en/<ファイル>.ts に訳を足す。npm run check-i18n が抜けと余りを検査する。
const parts = import.meta.glob<{ default: Record<string, string> }>('./en/*.ts', { eager: true });

export const EN: Record<string, string> = Object.assign({}, ...Object.values(parts).map((m) => m.default));
